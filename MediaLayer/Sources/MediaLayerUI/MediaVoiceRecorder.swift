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
  func start(file: URL, maxDurationMs: UInt64, onLimit: @escaping @Sendable () -> Void, onFailure: @escaping @Sendable (Error) -> Void) async throws
  func start(file: URL, maxDurationMs: UInt64, ticket: RecordingTicket, onLimit: @escaping @Sendable () -> Void, onFailure: @escaping @Sendable (Error) -> Void) async throws
  func peak() -> Float
  func stop() throws
  func finish() async throws
}

extension VoiceCapture {
  func finish() async throws { try stop() }

  func start(file: URL, maxDurationMs: UInt64, ticket: RecordingTicket, onLimit: @escaping @Sendable () -> Void, onFailure: @escaping @Sendable (Error) -> Void) async throws {
    try ticket.start {}
    try await start(file: file, maxDurationMs: maxDurationMs, onLimit: onLimit, onFailure: onFailure)
  }
}

/// Records a voice message into an m4a file (AAC, mono). `levels` gives the wave level 0 to 1 while a recording runs and nothing in rest.
/// The audio focus is taken for recording through `Media.audio` and given back when the recording ends. A refused permission, a busy microphone
/// and an empty recording are a `MediaError.Voice` with the text of the core. The recording ends by itself after `maxDuration`.
public final class MediaVoiceRecorder: @unchecked Sendable {
  private final class Running: @unchecked Sendable {
    let file: URL
    let startedAt: UInt64
    var take: UInt64 = 0
    var poll: Task<Void, Never>?
    var finish: Task<Result<VoiceRecording, Error>, Never>?
    init(file: URL, startedAt: UInt64) {
      self.file = file
      self.startedAt = startedAt
    }
  }

  private let capture: VoiceCapture
  private let maxDurationMs: UInt64
  private let minTakeMs: UInt64
  private let link: @Sendable () -> AudioLink?
  private let clock: @Sendable () -> UInt64
  private let waitForLevel: @Sendable () async -> Void
  private let lock = NSLock()
  private var running: Running?
  private var finishing: Task<Result<VoiceRecording, Error>, Never>?
  private var ended: Result<VoiceRecording, Error>?
  private var held: RecordingTicket?
  private var startPhase = StartPhase.free
  private enum StartEnd {
    case limit
    case failed(Error)
  }

  /// The end of the capture before the start committed is left here and belongs to the start.
  private enum StartPhase {
    case free
    case starting
    case ended(StartEnd)
  }

  private let awake: ScreenAwakeHold
  private var sinks: [UUID: AsyncStream<Float>.Continuation] = [:]
  private var voiceState = VoiceState(take: 0, phase: .idle, outcome: nil, recordedMs: 0)
  private var stateSinks: [UUID: AsyncStream<VoiceState>.Continuation] = [:]

  public var state: VoiceState { lock.withLock { voiceState } }
  public var states: AsyncStream<VoiceState> {
    AsyncStream { continuation in
      let id = UUID()
      lock.withLock { stateSinks[id] = continuation; continuation.yield(voiceState) }
      continuation.onTermination = { [weak self] _ in self?.lock.withLock { self?.stateSinks[id] = nil } }
    }
  }

  private func setLocked(_ value: VoiceState) {
    voiceState = value
    stateSinks.values.forEach { $0.yield(value) }
    awake.set(playing: value.phase != .idle)
  }

  private func publish(_ value: VoiceState) {
    lock.withLock { setLocked(value) }
    TestChange.fire()
  }

  public convenience init(maxDuration: TimeInterval? = nil, minTakeMs: UInt64 = 0) {
    self.init(capture: SystemVoiceCapture(), maxDurationMs: voiceLimitMs(requested: maxDuration.map { UInt64(max($0, 0) * 1000) }), minTakeMs: minTakeMs, screen: .shared)
  }

  deinit {
    running?.poll?.cancel()
    if let file = running?.file {
      try? capture.stop()
      removeFile(file)
    }
    held?.end()
  }

  init(
    capture: VoiceCapture,
    maxDurationMs: UInt64,
    minTakeMs: UInt64 = 0,
    link: @escaping @Sendable () -> AudioLink? = { Media.link },
    clock: @escaping @Sendable () -> UInt64 = { DispatchTime.now().uptimeNanoseconds / 1_000_000 },
    levelInterval: Duration = .milliseconds(50),
    waitForLevel: (@Sendable () async -> Void)? = nil,
    screen: ScreenAwake = .shared
  ) {
    awake = ScreenAwakeHold(kind: .always, awake: screen)
    self.capture = capture
    self.maxDurationMs = maxDurationMs
    self.minTakeMs = minTakeMs
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
      guard running == nil, finishing == nil, case .free = startPhase else { throw MediaError.Voice(detail: voiceRefusalText(reason: .microphoneBusy), reason: .microphoneBusy) }
      startPhase = .starting
      ended = nil
    }
    defer { lock.withLock { startPhase = .free } }
    guard let link = link() else { throw MediaError.Closed }
    let hold: RecordingTicket
    switch await link.admit(.voice) {
    case let .success(ticket): hold = ticket
    case let .failure(error):
      if let reason = error as? RecordingRefusal {
        let refusal = Self.refusal(reason)
        publishRefusal(refusal)
        throw refusal
      }
      throw error
    }
    let current: Running
    let early: Bool
    do {
      try Task.checkCancellation()
      try await capture.start(file: file, maxDurationMs: maxDurationMs, ticket: hold, onLimit: { [weak self] in self?.limitReached() }, onFailure: { [weak self] error in self?.captureFailed(error) })
      try Task.checkCancellation()
      // The clock of the take starts with its first PCM: `start` returns after it, not before the hardware opened.
      current = Running(file: file, startedAt: clock())
      do {
        try hold.start {}
      } catch {
        do { try capture.stop() } catch { Media.recordNativeFailure(error, source: .recorder) }
        throw error
      }
      let waitForLevel = waitForLevel
      current.poll = Task { [weak self] in
        while !Task.isCancelled {
          await waitForLevel()
          if Task.isCancelled { return }
          self?.publishLevel()
        }
      }
      do { early = try commit(current, hold) } catch {
        current.poll?.cancel()
        throw error
      }
    } catch let reason as RecordingRefusal {
      hold.end()
      removeFile(file)
      let refusal = Self.refusal(reason)
      publishRefusal(refusal)
      throw refusal
    } catch let refused as VoiceRefused {
      hold.end()
      removeFile(file)
      let refusal = MediaError.Voice(detail: voiceRefusalText(reason: refused.reason), reason: refused.reason)
      publishRefusal(refusal)
      throw refusal
    } catch {
      if !(error is CancellationError), !(error is MediaError) {
        let native = error as NSError
        let failure = NativeFailure(source: .recorder, domain: native.domain, code: Int64(native.code), message: native.localizedDescription)
        Media.publishFailure(failure)
        publish(VoiceState(take: state.take, phase: .idle, outcome: .failed(failure: failure), recordedMs: 0))
      }
      do { try capture.stop() } catch { Media.recordNativeFailure(error, source: .recorder) }
      hold.end()
      removeFile(file)
      throw error
    }
    TestChange.fire()
    if early { Task { _ = await finish(current) } }
    Task { [weak self] in
      for await reason in hold.revoked {
        guard let self else { return }
        _ = await self.finish(current, interrupted: Self.refusal(reason))
      }
    }
  }

  /// Makes the take running and publishes it in one step. A failure of the capture before it is the result of the start, a limit before it ends the take at once after it.
  private func commit(_ current: Running, _ hold: RecordingTicket) throws -> Bool {
    try lock.withLock {
      var limited = false
      if case let .ended(end) = startPhase {
        switch end {
        case let .failed(error): throw error
        case .limit: limited = true
        }
      }
      startPhase = .free
      held = hold
      running = current
      current.take = voiceState.take &+ 1
      setLocked(VoiceState(take: current.take, phase: .recording, outcome: nil, recordedMs: 0))
      return limited
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
    return MediaError.Voice(detail: voiceRefusalText(reason: reason), reason: reason)
  }

  private func publishRefusal(_ error: MediaError) {
    guard case let .Voice(_, reason) = error else { return }
    publish(VoiceState(take: state.take, phase: .idle, outcome: .refused(reason: reason), recordedMs: 0))
  }

  private func removeFile(_ file: URL) {
    NativeFailures.removeFile(file)
  }

  /// Ends the recording and gives the file. After the limit ended it, gives the outcome of that recording.
  public func stop() async throws -> VoiceRecording {
    let state = lock.withLock { (running, finishing, ended) }
    if let current = state.0 { return try await finish(current).get() }
    if let task = state.1 { return try await task.value.get() }
    guard let outcome = state.2 else { throw MediaError.Voice(detail: voiceRefusalText(reason: .empty), reason: .empty) }
    return try outcome.get()
  }

  private func limitReached() {
    let current = lock.withLock { () -> Running? in
      if running == nil, case .starting = startPhase { startPhase = .ended(.limit) }
      return running
    }
    guard let current else { return }
    Task { _ = await finish(current) }
  }

  private func captureFailed(_ error: Error) {
    let current = lock.withLock { () -> Running? in
      if running == nil, case .starting = startPhase { startPhase = .ended(.failed(error)) }
      return running
    }
    guard let current else { return }
    let refusal = (error as? RecordingRefusal).map(Self.refusal)
    Task { _ = await finish(current, interrupted: refusal, failure: refusal == nil ? error : nil) }
  }

  private func finish(_ current: Running, interrupted: MediaError? = nil, failure: Error? = nil) async -> Result<VoiceRecording, Error> {
    let task = lock.withLock {
      if let task = current.finish { return task }
      running = nil
      current.poll?.cancel()
      let session = held
      held = nil
      let task = Task { await self.finishRecording(current, session: session, interrupted: interrupted, failure: failure) }
      current.finish = task
      finishing = task
      return task
    }
    TestChange.fire()
    return await task.value
  }

  private func finishRecording(_ current: Running, session: RecordingTicket?, interrupted: MediaError?, failure: Error?) async -> Result<VoiceRecording, Error> {
    let length = clock() &- current.startedAt
    var outcome: Result<VoiceRecording, Error>
    if let failure { Media.recordNativeFailure(failure, source: .recorder) }
    do {
      do {
        if interrupted != nil || failure != nil { try capture.stop() } else { try await capture.finish() }
      } catch let refused as VoiceRefused where refused.reason == .empty && voiceTooShort(durationMs: length, minTakeMs: minTakeMs) && interrupted == nil && failure == nil {
        throw VoiceRefused(reason: .tooShort)
      } catch {
        if !(error is CancellationError), !(error is VoiceRefused), !(error is RecordingRefusal) { Media.recordNativeFailure(error, source: .recorder) }
        if interrupted == nil { throw error }
      }
      if let interrupted { throw interrupted }
      if let failure { throw failure }
      let size = (try? FileManager.default.attributesOfItem(atPath: current.file.path)[.size] as? Int) ?? 0
      if voiceTooShort(durationMs: length, minTakeMs: minTakeMs) { throw MediaError.Voice(detail: voiceRefusalText(reason: .tooShort), reason: .tooShort) }
      if length == 0 || size == 0 { throw MediaError.Voice(detail: voiceRefusalText(reason: .empty), reason: .empty) }
      outcome = .success(VoiceRecording(url: current.file, durationMs: length))
    } catch let refused as VoiceRefused {
      outcome = .failure(MediaError.Voice(detail: voiceRefusalText(reason: refused.reason), reason: refused.reason))
    } catch {
      outcome = .failure(error)
    }
    session?.end()
    let fileFailure: NativeFailure?
    if case .failure = outcome { fileFailure = NativeFailures.deleteFile(current.file) }
    else { fileFailure = nil }
    let result: VoiceOutcome
    switch outcome {
    case let .success(recording): result = .recorded(path: recording.url.path, durationMs: recording.durationMs)
    case let .failure(error):
      if let error = error as? MediaError, case let .Voice(_, reason) = error { result = .refused(reason: reason) }
      else {
        let native = error as NSError
        result = .failed(failure: NativeFailure(source: .recorder, domain: native.domain, code: Int64(native.code), message: native.localizedDescription))
      }
    }
    // The end of the take, its outcome and the room for the next start are one step: a start cannot commit between them.
    let finished = lock.withLock { () -> [AsyncStream<Float>.Continuation] in
      ended = outcome
      finishing = nil
      setLocked(VoiceState(take: current.take, phase: .idle, outcome: result, recordedMs: 0))
      defer { sinks.removeAll() }
      return Array(sinks.values)
    }
    TestChange.fire()
    for sink in finished { sink.finish() }
    if let fileFailure { NativeFailures.publish(fileFailure) }
    return outcome
  }

  private func publishLevel() {
    let level = voiceLevel(peak: capture.peak())
    let listeners = lock.withLock { () -> [AsyncStream<Float>.Continuation] in
      guard let current = running, voiceState.phase == .recording else { return [] }
      voiceState = VoiceState(take: voiceState.take, phase: .recording, outcome: nil, recordedMs: clock() &- current.startedAt)
      stateSinks.values.forEach { $0.yield(voiceState) }
      return Array(sinks.values)
    }
    for sink in listeners { sink.yield(level) }
  }
}

/// The microphone of a voice take through the input owner of the audio core. `start` returns with the first live PCM and a started writer: until then no file exists.
/// A take whose source was lost closes its owner at once, like every other end of a take; the next start opens a new one.
final class SystemVoiceCapture: VoiceCapture, @unchecked Sendable {
  private enum State {
    case idle
    case taking(VoiceTake, InputBox)
  }

  private let source: VoicePCMSource
  private let link: @Sendable () -> AudioLink?
  private let makeWriter: @Sendable (URL) -> VoiceFileWriter
  private let lock = NSLock()
  private var state = State.idle

  init(source: VoicePCMSource = AVCaptureVoiceSource(), link: @escaping @Sendable () -> AudioLink? = { Media.link }, writer: @escaping @Sendable (URL) -> VoiceFileWriter = { AssetVoiceWriter(url: $0) }) {
    self.source = source
    self.link = link
    makeWriter = writer
  }

  deinit { release(state) }

  static var settings: [String: Any] {
    let encoder = voiceEncoderSettings()
    return [
      AVFormatIDKey: Int(kAudioFormatMPEG4AAC),
      AVNumberOfChannelsKey: Int(encoder.channels),
      AVSampleRateKey: Int(encoder.sampleRateHz),
      AVEncoderBitRateKey: Int(encoder.bitRate)
    ]
  }

  func start(file: URL, maxDurationMs: UInt64, onLimit: @escaping @Sendable () -> Void, onFailure: @escaping @Sendable (Error) -> Void) async throws {
    try await run(file: file, maxDurationMs: maxDurationMs, ticket: nil, onLimit: onLimit, onFailure: onFailure)
  }

  func start(file: URL, maxDurationMs: UInt64, ticket: RecordingTicket, onLimit: @escaping @Sendable () -> Void, onFailure: @escaping @Sendable (Error) -> Void) async throws {
    try ticket.start {}
    try await run(file: file, maxDurationMs: maxDurationMs, ticket: ticket, onLimit: onLimit, onFailure: onFailure)
  }

  private func run(file: URL, maxDurationMs: UInt64, ticket: RecordingTicket?, onLimit: @escaping @Sendable () -> Void, onFailure: @escaping @Sendable (Error) -> Void) async throws {
    guard let audio = link() else { throw MediaError.Closed }
    let first = FirstPCM()
    let box = InputBox()
    let holder = TakeHolder()
    let take = VoiceTake(
      url: file, writer: makeWriter(file), settings: Self.settings, maxDurationMs: maxDurationMs,
      onStarted: { first.settle(.success(())) },
      onLimit: onLimit,
      onBroken: { [weak self, holder] broken in
        let error = self?.ended(holder.take, by: broken) ?? Self.loss(nil)
        if !first.settle(.failure(error)) { onFailure(error) }
      }
    )
    holder.take = take
    let microphone = VoiceMicrophone(source: source)
    microphone.attach(take)
    box.microphone = microphone
    // The take and its input are registered before the owner can act: the owner opens the input and its first word may come at once.
    release(lock.withLock { () -> State in
      defer { state = .taking(take, box) }
      return state
    })
    box.set(CameraAudioInput(link: audio, microphone: microphone, kind: .voice, failureSource: nil) { _ in })
    ticket?.whenRevoked { reason in _ = first.settle(.failure(reason)) }
    do {
      try await withTaskCancellationHandler { try await first.wait() } onCancel: { _ = first.settle(.failure(CancellationError())) }
    } catch {
      abort(take)
      throw error
    }
  }

  func peak() -> Float {
    guard case let .taking(take, _) = lock.withLock({ state }) else { return 0 }
    return take.takePeak()
  }

  func stop() throws {
    guard let take = leaveTaking() else { return }
    take.cancel()
  }

  func finish() async throws {
    guard let take = leaveTaking() else { return }
    try await take.finish()
  }

  private func leaveTaking() -> VoiceTake? {
    let taking = lock.withLock { () -> (VoiceTake, InputBox)? in
      guard case let .taking(take, box) = state else { return nil }
      state = .idle
      return (take, box)
    }
    guard let (take, box) = taking else { return nil }
    box.close()
    return take
  }

  private func abort(_ take: VoiceTake) {
    let box = lock.withLock { () -> InputBox? in
      guard case let .taking(current, box) = state, current === take else { return nil }
      state = .idle
      return box
    }
    guard let box else { return }
    take.cancel()
    box.close()
  }

  /// The one place a take that ended by itself is told. A lost source and a failed writer both close the input.
  private func ended(_ take: VoiceTake?, by broken: VoiceTake.Broken) -> Error {
    let box = lock.withLock { () -> InputBox? in
      guard let take, case let .taking(current, box) = state, current === take else { return nil }
      state = .idle
      return box
    }
    box?.microphone?.attach(nil)
    switch broken {
    case let .lost(error):
      box?.close()
      return Self.loss(error)
    case let .failed(error):
      box?.close()
      return error
    }
  }

  private func release(_ state: State) {
    switch state {
    case .idle: break
    case let .taking(take, box):
      take.cancel()
      box.close()
    }
  }

  private static func loss(_ error: Error?) -> Error {
    guard let error else { return RecordingRefusal.microphoneSilent }
    switch microphoneRefusal(error) {
    case .permissionDenied: return RecordingRefusal.permissionDenied
    case .microphoneBusy: return RecordingRefusal.microphoneBusy
    default: return error
    }
  }
}

/// The one lifetime of the input of a take: it exists before its owner does and closes the owner whenever it comes.
private final class InputBox: @unchecked Sendable {
  private let lock = NSLock()
  private var owner: CameraAudioInput?
  private var closed = false
  var microphone: VoiceMicrophone?

  func set(_ owner: CameraAudioInput) {
    let late = lock.withLock { () -> Bool in
      if closed { return true }
      self.owner = owner
      return false
    }
    if late { owner.close() }
  }

  func close() {
    let owner = lock.withLock { () -> CameraAudioInput? in
      closed = true
      defer { self.owner = nil }
      return self.owner
    }
    owner?.close()
  }
}

private final class TakeHolder: @unchecked Sendable {
  private let lock = NSLock()
  private var value: VoiceTake?
  var take: VoiceTake? {
    get { lock.withLock { value } }
    set { lock.withLock { value = newValue } }
  }
}

/// The wait of a start for the first live PCM: one result, the continuation resumes exactly once whoever settles first.
private final class FirstPCM: @unchecked Sendable {
  private let lock = NSLock()
  private var result: Result<Void, Error>?
  private var continuation: CheckedContinuation<Void, Error>?

  @discardableResult
  func settle(_ value: Result<Void, Error>) -> Bool {
    let (first, pending) = lock.withLock { () -> (Bool, CheckedContinuation<Void, Error>?) in
      guard result == nil else { return (false, nil) }
      result = value
      defer { continuation = nil }
      return (true, continuation)
    }
    pending?.resume(with: value)
    return first
  }

  func wait() async throws {
    try await withCheckedThrowingContinuation { (waiting: CheckedContinuation<Void, Error>) in
      let ready = lock.withLock { () -> Result<Void, Error>? in
        if let result { return result }
        continuation = waiting
        return nil
      }
      if let ready { waiting.resume(with: ready) }
    }
  }
}
