@preconcurrency import AVFoundation
import Foundation
import MediaLayer

/// A finished voice recording: an m4a file (AAC, mono) and its length.
public struct VoiceRecording: Sendable, Equatable {
  public let url: URL
  public let durationMs: UInt64
}

struct VoiceRefused: Error {
  let reason: VoiceRefusal
}

/// The microphone side of a recording. `start` throws `VoiceRefused`, `peak` is the linear peak 0 to 1.
protocol VoiceCapture: Sendable {
  func start(file: URL, maxDurationMs: UInt64, onLimit: @escaping @Sendable () -> Void) async throws
  func peak() -> Float
  func stop() throws
}

/// Records a voice message into an m4a file (AAC, mono). `levels` gives the wave level 0 to 1 while a recording runs and nothing in rest.
/// The audio focus is taken for recording through `Media.audio` and given back when the recording ends. A refused permission, a busy microphone
/// and an empty recording are a `MediaError.Voice` with the text of the core. The recording ends by itself after `maxDuration`.
public final class MediaVoiceRecorder: @unchecked Sendable {
  private final class Running: @unchecked Sendable {
    let file: URL
    let startedAt: UInt64
    var poll: Task<Void, Never>?
    var finish: Task<Result<VoiceRecording, Error>, Never>?
    init(file: URL, startedAt: UInt64) {
      self.file = file
      self.startedAt = startedAt
    }
  }

  private let capture: VoiceCapture
  private let maxDurationMs: UInt64
  private let link: @Sendable () -> AudioLink?
  private let clock: @Sendable () -> UInt64
  private let waitForLevel: @Sendable () async -> Void
  private let lock = NSLock()
  private var running: Running?
  private var finishing: Task<Result<VoiceRecording, Error>, Never>?
  private var ended: Result<VoiceRecording, Error>?
  private var held: RecordingTicket?
  private var starting = false
  private var sinks: [UUID: AsyncStream<Float>.Continuation] = [:]

  public convenience init(maxDuration: TimeInterval? = nil) {
    self.init(capture: SystemVoiceCapture(), maxDurationMs: voiceLimitMs(requested: maxDuration.map { UInt64(max($0, 0) * 1000) }))
  }

  deinit {
    running?.poll?.cancel()
    if let file = running?.file {
      try? capture.stop()
      try? FileManager.default.removeItem(at: file)
    }
    held?.end()
  }

  init(
    capture: VoiceCapture,
    maxDurationMs: UInt64,
    link: @escaping @Sendable () -> AudioLink? = { Media.link },
    clock: @escaping @Sendable () -> UInt64 = { DispatchTime.now().uptimeNanoseconds / 1_000_000 },
    levelInterval: Duration = .milliseconds(50),
    waitForLevel: (@Sendable () async -> Void)? = nil
  ) {
    self.capture = capture
    self.maxDurationMs = maxDurationMs
    self.link = link
    self.clock = clock
    self.waitForLevel = waitForLevel ?? { try? await Task.sleep(for: levelInterval) }
  }

  public var isRecording: Bool { lock.withLock { running != nil } }

  var levelPoll: Task<Void, Never>? { lock.withLock { running?.poll } }

  /// The wave levels of the recording that runs. The stream ends when the observer stops listening.
  public var levels: AsyncStream<Float> {
    let id = UUID()
    return AsyncStream { continuation in
      lock.withLock { sinks[id] = continuation }
      continuation.onTermination = { [weak self] _ in self?.lock.withLock { self?.sinks[id] = nil } }
    }
  }

  public func start(to file: URL) async throws {
    try lock.withLock {
      guard running == nil && finishing == nil && !starting else { throw MediaError.Unexpected(detail: "a recording is already running") }
      starting = true
      ended = nil
    }
    defer { lock.withLock { starting = false } }
    guard let link = link() else { throw MediaError.Closed }
    let hold: RecordingTicket
    switch await link.admit(.voice) {
    case let .success(ticket): hold = ticket
    case let .failure(reason): throw Self.refusal(reason)
    }
    do {
      try await capture.start(file: file, maxDurationMs: maxDurationMs) { [weak self] in self?.limitReached() }
    } catch let refused as VoiceRefused {
      hold.end()
      try? FileManager.default.removeItem(at: file)
      throw MediaError.Voice(detail: voiceRefusalText(reason: refused.reason))
    } catch {
      hold.end()
      try? FileManager.default.removeItem(at: file)
      throw error
    }
    let current = Running(file: file, startedAt: clock())
    let waitForLevel = waitForLevel
    current.poll = Task { [weak self] in
      while !Task.isCancelled {
        await waitForLevel()
        if Task.isCancelled { return }
        self?.publishLevel()
      }
    }
    lock.withLock {
      held = hold
      running = current
    }
    Task { [weak self] in
      for await reason in hold.revoked {
        guard let self else { return }
        _ = await self.finish(current, interrupted: Self.refusal(reason))
      }
    }
  }

  private static func refusal(_ refusal: RecordingRefusal) -> MediaError {
    let reason: VoiceRefusal
    switch refusal {
    case .inCall: reason = .inCall
    case .interrupted: reason = .interrupted
    case .microphoneSilent: reason = .microphoneSilent
    case .permissionDenied: reason = .permissionDenied
    case .microphoneBusy: reason = .microphoneBusy
    }
    return MediaError.Voice(detail: voiceRefusalText(reason: reason))
  }

  /// Ends the recording and gives the file. After the limit ended it, gives the outcome of that recording.
  public func stop() async throws -> VoiceRecording {
    let state = lock.withLock { (running, finishing, ended) }
    if let current = state.0 { return try await finish(current).get() }
    if let task = state.1 { return try await task.value.get() }
    guard let outcome = state.2 else { throw MediaError.Unexpected(detail: "no recording was started") }
    return try outcome.get()
  }

  private func limitReached() {
    guard let current = lock.withLock({ running }) else { return }
    Task { _ = await finish(current) }
  }

  private func finish(_ current: Running, interrupted: MediaError? = nil) async -> Result<VoiceRecording, Error> {
    let task = lock.withLock {
      if let task = current.finish { return task }
      running = nil
      current.poll?.cancel()
      let session = held
      held = nil
      let task = Task { await self.finishRecording(current, session: session, interrupted: interrupted) }
      current.finish = task
      finishing = task
      return task
    }
    TestChange.fire()
    return await task.value
  }

  private func finishRecording(_ current: Running, session: RecordingTicket?, interrupted: MediaError?) async -> Result<VoiceRecording, Error> {
    let length = clock() &- current.startedAt
    var outcome: Result<VoiceRecording, Error>
    do {
      do {
        try capture.stop()
      } catch where interrupted != nil {}
      if let interrupted { throw interrupted }
      let size = (try? FileManager.default.attributesOfItem(atPath: current.file.path)[.size] as? Int) ?? 0
      if length == 0 || size == 0 { throw MediaError.Voice(detail: voiceRefusalText(reason: .empty)) }
      outcome = .success(VoiceRecording(url: current.file, durationMs: length))
    } catch let refused as VoiceRefused {
      outcome = .failure(MediaError.Voice(detail: voiceRefusalText(reason: refused.reason)))
    } catch {
      outcome = .failure(error)
    }
    session?.end()
    if case .failure = outcome { try? FileManager.default.removeItem(at: current.file) }
    let finished = lock.withLock { () -> [AsyncStream<Float>.Continuation] in
      ended = outcome
      finishing = nil
      defer { sinks.removeAll() }
      return Array(sinks.values)
    }
    for sink in finished { sink.finish() }
    return outcome
  }

  private func publishLevel() {
    let level = voiceLevel(peak: capture.peak())
    for sink in lock.withLock({ Array(sinks.values) }) { sink.yield(level) }
  }
}

final class SystemVoiceCapture: NSObject, VoiceCapture, AVAudioRecorderDelegate, @unchecked Sendable {
  private let lock = NSLock()
  private var recorder: AVAudioRecorder?
  private var onLimit: (@Sendable () -> Void)?

  static var settings: [String: Any] {
    let encoder = voiceEncoderSettings()
    return [
      AVFormatIDKey: Int(kAudioFormatMPEG4AAC),
      AVNumberOfChannelsKey: Int(encoder.channels),
      AVSampleRateKey: Int(encoder.sampleRateHz),
      AVEncoderBitRateKey: Int(encoder.bitRate)
    ]
  }

  func start(file: URL, maxDurationMs: UInt64, onLimit: @escaping @Sendable () -> Void) async throws {
    guard await Self.permitted() else { throw VoiceRefused(reason: .permissionDenied) }
    let made: AVAudioRecorder
    made = try AVAudioRecorder(url: file, settings: Self.settings)
    made.delegate = self
    made.isMeteringEnabled = true
    guard made.record(forDuration: TimeInterval(maxDurationMs) / 1000) else { throw NSError(domain: "MediaLayer.VoiceCapture", code: 1) }
    lock.withLock {
      recorder = made
      self.onLimit = onLimit
    }
  }

  func peak() -> Float {
    guard let recorder = lock.withLock({ recorder }) else { return 0 }
    recorder.updateMeters()
    return powf(10, recorder.peakPower(forChannel: 0) / 20)
  }

  func stop() throws {
    guard let made = lock.withLock({ () -> AVAudioRecorder? in
      defer { recorder = nil }
      return recorder
    }) else { return }
    made.stop()
  }

  func audioRecorderDidFinishRecording(_ recorder: AVAudioRecorder, successfully flag: Bool) {
    let limit = lock.withLock { () -> (@Sendable () -> Void)? in
      defer { onLimit = nil }
      return self.recorder === recorder ? onLimit : nil
    }
    limit?()
  }

  private static func permitted() async -> Bool {
    switch AVAudioApplication.shared.recordPermission {
    case .granted: return true
    case .undetermined: return await AVAudioApplication.requestRecordPermission()
    default: return false
    }
  }
}
