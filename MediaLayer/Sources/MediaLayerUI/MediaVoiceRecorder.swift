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
  private final class Running {
    let file: URL
    let startedAt: UInt64
    var poll: Task<Void, Never>?
    init(file: URL, startedAt: UInt64) {
      self.file = file
      self.startedAt = startedAt
    }
  }

  private let capture: VoiceCapture
  private let maxDurationMs: UInt64
  private let audio: @Sendable () -> MediaAudioSession?
  private let clock: @Sendable () -> UInt64
  private let levelInterval: Duration
  private let lock = NSLock()
  private var running: Running?
  private var ended: Result<VoiceRecording, Error>?
  private var held: MediaAudioSession?
  private var sinks: [UUID: AsyncStream<Float>.Continuation] = [:]

  public convenience init(maxDuration: TimeInterval? = nil) {
    self.init(capture: SystemVoiceCapture(), maxDurationMs: voiceLimitMs(requested: maxDuration.map { UInt64(max($0, 0) * 1000) }))
  }

  init(
    capture: VoiceCapture,
    maxDurationMs: UInt64,
    audio: @escaping @Sendable () -> MediaAudioSession? = { Media.audio },
    clock: @escaping @Sendable () -> UInt64 = { DispatchTime.now().uptimeNanoseconds / 1_000_000 },
    levelInterval: Duration = .milliseconds(50)
  ) {
    self.capture = capture
    self.maxDurationMs = maxDurationMs
    self.audio = audio
    self.clock = clock
    self.levelInterval = levelInterval
  }

  public var isRecording: Bool { lock.withLock { running != nil } }

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
      guard running == nil else { throw MediaError.Unexpected(detail: "a recording is already running") }
      ended = nil
    }
    let session = audio()
    if let session, !(await session.activateRecording()) {
      throw MediaError.Voice(detail: voiceRefusalText(reason: .microphoneBusy))
    }
    do {
      try await capture.start(file: file, maxDurationMs: maxDurationMs) { [weak self] in self?.limitReached() }
    } catch let refused as VoiceRefused {
      await session?.deactivateRecording()
      try? FileManager.default.removeItem(at: file)
      throw MediaError.Voice(detail: voiceRefusalText(reason: refused.reason))
    }
    let current = Running(file: file, startedAt: clock())
    current.poll = Task { [weak self] in await self?.pollLevels() }
    lock.withLock {
      held = session
      running = current
    }
  }

  /// Ends the recording and gives the file. After the limit ended it, gives the outcome of that recording.
  public func stop() async throws -> VoiceRecording {
    guard let current = lock.withLock({ running }) else {
      guard let outcome = lock.withLock({ ended }) else { throw MediaError.Unexpected(detail: "no recording was started") }
      return try outcome.get()
    }
    return try await finish(current).get()
  }

  private func limitReached() {
    guard let current = lock.withLock({ running }) else { return }
    Task { _ = await finish(current) }
  }

  private func finish(_ current: Running) async -> Result<VoiceRecording, Error> {
    let taken: (session: MediaAudioSession?, outcome: Result<VoiceRecording, Error>?) = lock.withLock {
      guard running === current else { return (nil, ended) }
      running = nil
      current.poll?.cancel()
      let session = held
      held = nil
      return (session, nil)
    }
    if let outcome = taken.outcome { return outcome }
    let length = clock() &- current.startedAt
    var outcome: Result<VoiceRecording, Error>
    do {
      try capture.stop()
      let size = (try? FileManager.default.attributesOfItem(atPath: current.file.path)[.size] as? Int) ?? 0
      if length == 0 || size == 0 { throw MediaError.Voice(detail: voiceRefusalText(reason: .empty)) }
      outcome = .success(VoiceRecording(url: current.file, durationMs: length))
    } catch let refused as VoiceRefused {
      outcome = .failure(MediaError.Voice(detail: voiceRefusalText(reason: refused.reason)))
    } catch {
      outcome = .failure(error)
    }
    await taken.session?.deactivateRecording()
    if case .failure = outcome { try? FileManager.default.removeItem(at: current.file) }
    let finished = lock.withLock { () -> [AsyncStream<Float>.Continuation] in
      ended = outcome
      defer { sinks.removeAll() }
      return Array(sinks.values)
    }
    for sink in finished { sink.finish() }
    return outcome
  }

  private func pollLevels() async {
    while !Task.isCancelled {
      try? await Task.sleep(for: levelInterval)
      if Task.isCancelled { return }
      let level = voiceLevel(peak: capture.peak())
      for sink in lock.withLock({ Array(sinks.values) }) { sink.yield(level) }
    }
  }
}

final class SystemVoiceCapture: NSObject, VoiceCapture, AVAudioRecorderDelegate, @unchecked Sendable {
  private let lock = NSLock()
  private var recorder: AVAudioRecorder?
  private var onLimit: (@Sendable () -> Void)?

  static let settings: [String: Any] = [
    AVFormatIDKey: Int(kAudioFormatMPEG4AAC),
    AVNumberOfChannelsKey: 1,
    AVSampleRateKey: 44_100,
    AVEncoderBitRateKey: 64_000
  ]

  func start(file: URL, maxDurationMs: UInt64, onLimit: @escaping @Sendable () -> Void) async throws {
    guard await Self.permitted() else { throw VoiceRefused(reason: .permissionDenied) }
    let made: AVAudioRecorder
    do {
      made = try AVAudioRecorder(url: file, settings: Self.settings)
    } catch {
      throw VoiceRefused(reason: .microphoneBusy)
    }
    made.delegate = self
    made.isMeteringEnabled = true
    guard made.record(forDuration: TimeInterval(maxDurationMs) / 1000) else { throw VoiceRefused(reason: .microphoneBusy) }
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
