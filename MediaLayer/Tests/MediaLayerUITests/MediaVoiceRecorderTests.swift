import Foundation
import AVFoundation
import MediaLayer
import Testing
@testable import MediaLayerUI

private final class FakeCapture: VoiceCapture, @unchecked Sendable {
  private let lock = NSLock()
  private let refusal: VoiceRefusal?
  private let peaks: [Float]
  private let writes: Bool
  private var file: URL?
  private var readCount = 0
  let readEvents = AsyncStream<Void>.makeStream()
  private(set) var limitMs: UInt64 = 0
  private(set) var limit: (@Sendable () -> Void)?
  private(set) var failure: (@Sendable (Error) -> Void)?

  init(refusal: VoiceRefusal? = nil, peaks: [Float] = [0.5], writes: Bool = true) {
    self.refusal = refusal
    self.peaks = peaks
    self.writes = writes
  }

  var reads: Int { lock.withLock { readCount } }

  func start(file: URL, maxDurationMs: UInt64, onLimit: @escaping @Sendable () -> Void, onFailure: @escaping @Sendable (Error) -> Void) async throws {
    if let refusal { throw VoiceRefused(reason: refusal) }
    lock.withLock {
      self.file = file
      limitMs = maxDurationMs
      limit = onLimit
      failure = onFailure
    }
    TestSignal.shared.fire()
  }

  func peak() -> Float {
    let peak = lock.withLock {
      defer { readCount += 1 }
      return peaks[readCount % peaks.count]
    }
    readEvents.continuation.yield(())
    TestSignal.shared.fire()
    return peak
  }

  private var stopCount = 0
  var stops: Int { lock.withLock { stopCount } }

  func stop() throws {
    lock.withLock { stopCount += 1 }
    TestSignal.shared.fire()
    if writes, let file = lock.withLock({ file }) { try Data([1, 2, 3]).write(to: file) }
  }
}

private final class FirstStartGate: VoiceCapture, @unchecked Sendable {
  private let lock = NSLock()
  private var calls = 0
  private var file: URL?
  private var opened: CheckedContinuation<Void, Never>?
  private var isOpen = false
  let entered = AsyncStream<Void>.makeStream()

  func start(file: URL, maxDurationMs: UInt64, onLimit: @escaping @Sendable () -> Void, onFailure: @escaping @Sendable (Error) -> Void) async throws {
    let first = lock.withLock { () -> Bool in
      calls += 1
      if calls == 1 { self.file = file }
      return calls == 1
    }
    guard first else { return }
    entered.continuation.yield(())
    await withCheckedContinuation { continuation in
      let resume = lock.withLock { () -> Bool in
        if isOpen { return true }
        opened = continuation
        return false
      }
      if resume { continuation.resume() }
    }
  }

  func open() {
    let waiting = lock.withLock { () -> CheckedContinuation<Void, Never>? in
      isOpen = true
      defer { opened = nil }
      return opened
    }
    waiting?.resume()
  }

  func peak() -> Float { 0.5 }

  func stop() throws {
    if let file = lock.withLock({ file }) { try Data([1, 2, 3]).write(to: file) }
  }
}

/// Records what the session port had applied at the moment the recorder hardware stops.
private final class StopWitness: VoiceCapture, @unchecked Sendable {
  private let lock = NSLock()
  private let session: FakeSessionPort
  private var file: URL?
  private(set) var appliedAtStop: [String]?

  init(session: FakeSessionPort) { self.session = session }

  func start(file: URL, maxDurationMs: UInt64, onLimit: @escaping @Sendable () -> Void, onFailure: @escaping @Sendable (Error) -> Void) async throws {
    lock.withLock { self.file = file }
    try Data([1, 2, 3]).write(to: file)
  }

  func peak() -> Float { 0.5 }

  func stop() throws {
    let applied = session.log
    lock.withLock { appliedAtStop = applied }
    TestSignal.shared.fire()
  }
}

/// The recorder hardware fails with a native error at start or at stop.
private final class FailingCapture: VoiceCapture, @unchecked Sendable {
  private let startError: Error?
  private let stopError: Error?

  init(start: Error? = nil, stop: Error? = nil) {
    startError = start
    stopError = stop
  }

  func start(file: URL, maxDurationMs: UInt64, onLimit: @escaping @Sendable () -> Void, onFailure: @escaping @Sendable (Error) -> Void) async throws {
    if let startError { throw startError }
  }

  func peak() -> Float { 0.5 }

  func stop() throws {
    if let stopError { throw stopError }
  }
}

/// Holds the session port inside the first recording apply until the test opens it, so the admission waits.
private final class RecordingApplyGate: @unchecked Sendable {
  private let lock = NSLock()
  private var held = false
  private let gate = DispatchSemaphore(value: 0)
  let reached = AsyncStream<Void>.makeStream()

  func pass(_ plan: SessionPlan) {
    let first = lock.withLock { () -> Bool in
      guard plan.category == .playAndRecord, !held else { return false }
      held = true
      return true
    }
    guard first else { return }
    reached.continuation.yield(())
    gate.wait()
  }

  func open() { gate.signal() }
}

private final class StartOutcome: @unchecked Sendable {
  private let lock = NSLock()
  private var value: Result<Void, Error>?

  var result: Result<Void, Error>? { lock.withLock { value } }

  func set(_ result: Result<Void, Error>) {
    lock.withLock { value = result }
    TestSignal.shared.fire()
  }
}

private final class Ticks: @unchecked Sendable {
  private let lock = NSLock()
  private var value: UInt64 = 1000
  func read() -> UInt64 { lock.withLock { value } }
  func advance(_ ms: UInt64) { lock.withLock { value += ms } }
}

@Suite(.serialized, .timeLimit(.minutes(1)))
struct MediaVoiceRecorderTests {
  private func file() -> URL {
    FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).m4a")
  }

  private func makeRecorder(_ capture: VoiceCapture, ticks: Ticks = Ticks(), rig: TestAudioRig? = nil, maxMs: UInt64 = 60_000, minTakeMs: UInt64 = 0, screen: ScreenSwitch = ScreenSwitch()) -> MediaVoiceRecorder {
    let rig = rig ?? (try! TestAudioRig())
    return MediaVoiceRecorder(capture: capture, maxDurationMs: maxMs, minTakeMs: minTakeMs, link: { rig.link }, clock: { ticks.read() }, levelInterval: .milliseconds(5), screen: screen.awake())
  }

  @Test
  func inv_api_62_a_running_voice_take_keeps_the_screen_on() async throws {
    let screen = ScreenSwitch()
    let ticks = Ticks()
    let recorder = makeRecorder(FakeCapture(), ticks: ticks, screen: screen)
    #expect(!screen.on)

    try await recorder.start(to: file())
    await eventually("the screen stays on while the take runs") { screen.on }

    ticks.advance(1000)
    _ = try await recorder.stop()
    await eventually("the screen may sleep after the take") { !screen.on }
  }

  @Test
  func inv_audio_69_voice_state_publishes_recorded_time() async throws {
    let ticks = Ticks()
    let recorder = makeRecorder(FakeCapture(), ticks: ticks)
    let states = Collected(recorder.states)

    try await recorder.start(to: file())
    ticks.advance(1200)
    await eventually("the take shows its recorded time") { states.last?.recordedMs == 1200 }
    _ = try await recorder.stop()

    #expect(states.values.contains { $0.phase == .recording && $0.recordedMs == 1200 })
    #expect(recorder.state.phase == .idle && recorder.state.recordedMs == 0)
  }

  @Test
  func inv_audio_70_a_short_voice_take_is_refused_and_deleted() async throws {
    let ticks = Ticks()
    let recorder = makeRecorder(FakeCapture(), ticks: ticks, minTakeMs: 1000)
    let target = file()

    try await recorder.start(to: target)
    ticks.advance(400)
    do {
      _ = try await recorder.stop()
      Issue.record("a take shorter than the minimum was accepted")
    } catch let error as MediaError {
      guard case let .Voice(_, reason) = error else { Issue.record("wrong error \(error)"); return }
      #expect(reason == .tooShort)
    }

    #expect(recorder.state.outcome == .refused(reason: .tooShort))
    #expect(!FileManager.default.fileExists(atPath: target.path))
  }

  @Test
  func inv_voice_04_a_recording_gives_an_existing_file_and_a_length_above_zero() async throws {
    let ticks = Ticks()
    let recorder = makeRecorder(FakeCapture(), ticks: ticks)
    let target = file()

    try await recorder.start(to: target)
    ticks.advance(2500)
    let recording = try await recorder.stop()

    #expect(recording.url == target)
    #expect(FileManager.default.fileExists(atPath: target.path))
    #expect(recording.durationMs == 2500)
    #expect(!recorder.isRecording)
  }

  @Test
  func inv_voice_05_the_levels_follow_the_peaks_through_the_core_formula_and_stay_in_zero_to_one() async throws {
    let peaks: [Float] = [0, 0.03, 1, 5]
    let ticks = Ticks()
    let recorder = makeRecorder(FakeCapture(peaks: peaks), ticks: ticks)
    var iterator = recorder.levels.makeAsyncIterator()

    try await recorder.start(to: file())
    var seen: [Float] = []
    for _ in peaks { if let level = await iterator.next() { seen.append(level) } }
    ticks.advance(1000)
    _ = try await recorder.stop()

    #expect(seen == peaks.map { voiceLevel(peak: $0) })
    #expect(seen.allSatisfy { (0 ... 1).contains($0) })
  }

  @Test
  func inv_voice_06_the_level_is_not_read_in_rest_or_after_the_recording_ends() async throws {
    let capture = FakeCapture()
    let ticks = Ticks()
    let levelTicks = AsyncStream<Void>.makeStream()
    let rig = try TestAudioRig()
    let recorder = MediaVoiceRecorder(capture: capture, maxDurationMs: 60_000, link: { rig.link }, clock: { ticks.read() }, waitForLevel: {
      var iterator = levelTicks.stream.makeAsyncIterator()
      _ = await iterator.next()
    })
    var reads = capture.readEvents.stream.makeAsyncIterator()
    #expect(recorder.levelPoll.map { _ in true } == nil)
    #expect(capture.reads == 0)

    try await recorder.start(to: file())
    let poll = try #require(recorder.levelPoll)
    for _ in 0..<2 {
      levelTicks.continuation.yield(())
      _ = await reads.next()
    }
    ticks.advance(1000)
    _ = try await recorder.stop()
    await poll.value
    let after = capture.reads
    levelTicks.continuation.yield(())

    #expect(recorder.levelPoll.map { _ in true } == nil)
    #expect(capture.reads == after)
  }

  @Test
  func inv_voice_08_the_recording_takes_the_audio_focus_and_gives_it_back_at_the_end() async throws {
    let rig = try TestAudioRig()
    let port = rig.port
    let ticks = Ticks()
    let recorder = makeRecorder(FakeCapture(), ticks: ticks, rig: rig)

    try await recorder.start(to: file())
    #expect(port.log.last == "apply AVAudioSessionCategoryPlayAndRecord AVAudioSessionModeDefault activate=true")
    ticks.advance(1000)
    _ = try await recorder.stop()

    await eventually("recording released") { port.log.last == "deactivate" }
  }

  @Test
  func aSecondStartWhileTheFirstIsStartingIsRefusedAndHoldsNoRecordingSession() async throws {
    let rig = try TestAudioRig()
    let capture = FirstStartGate()
    let ticks = Ticks()
    let recorder = makeRecorder(capture, ticks: ticks, rig: rig)
    let first = Task { try await recorder.start(to: file()) }
    var entered = capture.entered.stream.makeAsyncIterator()
    _ = await entered.next()

    await #expect(throws: MediaError.self) { try await recorder.start(to: file()) }
    capture.open()
    try await first.value
    ticks.advance(1000)
    _ = try await recorder.stop()

    await eventually("recording released") { rig.port.log.last == "deactivate" }
  }

  @Test(arguments: [0, 4])
  func playbackAfterRecordingStartsWithAnActivePlaybackSession(busyDeactivations: Int) async throws {
    let port = FakeSessionPort(busyDeactivations: busyDeactivations)
    let rig = try TestAudioRig(port: port)
    let ticks = Ticks()
    let recorder = makeRecorder(FakeCapture(), ticks: ticks, rig: rig)
    let target = file()
    defer { try? FileManager.default.removeItem(at: target) }
    try await recorder.start(to: target)
    ticks.advance(1000)
    _ = try await recorder.stop()

    let factory = AVQueuePlayerSlotFactory(startMuted: false, audio: rig.audio)
    let slot = await factory.makeSlot(7)
    let clip = Bundle.module.url(forResource: "silent", withExtension: "mp4", subdirectory: "Fixtures")!
    await slot.load(key: "voice", route: PlaybackRoute(url: clip.absoluteString, headers: [:], generation: nil, hls: false, looping: true))
    await slot.play()
    for attempt in 0..<busyDeactivations {
      await eventually("deactivation busy") { port.log.filter { $0 == "deactivate BUSY" }.count > attempt }
      port.send(.routeChanged(.categoryChange, hadPrivateOutput: false))
    }
    await eventually("playback session restored") { port.log.last == "apply AVAudioSessionCategoryPlayback AVAudioSessionModeMoviePlayback activate=true" }
    let calls = port.log
    #expect(calls.last == "apply AVAudioSessionCategoryPlayback AVAudioSessionModeMoviePlayback activate=true")
    await slot.unload()
  }

  @Test
  func inv_voice_09_the_limit_ends_the_recording_by_itself_and_stop_gives_its_file() async throws {
    let ticks = Ticks()
    let capture = FakeCapture()
    let recorder = makeRecorder(capture, ticks: ticks, maxMs: 30_000)

    try await recorder.start(to: file())
    #expect(capture.limitMs == 30_000)
    ticks.advance(30_000)
    capture.limit?()
    let recording = try await recorder.stop()

    #expect(recording.durationMs == 30_000)
    #expect(FileManager.default.fileExists(atPath: recording.url.path))
    #expect(capture.stops == 1, "the recording ended once")
  }

  @Test
  func inv_voice_10_a_recording_without_sound_is_a_media_error_and_leaves_no_file() async throws {
    let ticks = Ticks()
    let recorder = makeRecorder(FakeCapture(writes: false), ticks: ticks)
    let target = file()

    try await recorder.start(to: target)
    ticks.advance(1000)
    do {
      _ = try await recorder.stop()
      Issue.record("an empty recording was accepted")
    } catch let error as MediaError {
      guard case let .Voice(detail, reason) = error else { Issue.record("wrong error \(error)"); return }
      #expect((detail, reason) == (voiceRefusalText(reason: .empty), .empty))
    }
    #expect(!FileManager.default.fileExists(atPath: target.path))
  }

  @Test
  func inv_voice_11_the_system_capture_records_aac_in_one_channel() {
    #expect(SystemVoiceCapture.settings[AVFormatIDKey] as? Int == Int(kAudioFormatMPEG4AAC))
    #expect(SystemVoiceCapture.settings[AVNumberOfChannelsKey] as? Int == 1)
  }

  @Test
  func inv_voice_12_the_system_capture_records_at_44100_hz_and_128000_bit_per_second() {
    #expect(SystemVoiceCapture.settings[AVSampleRateKey] as? Int == 44_100)
    #expect(SystemVoiceCapture.settings[AVNumberOfChannelsKey] as? Int == 1)
    #expect(SystemVoiceCapture.settings[AVEncoderBitRateKey] as? Int == 128_000)
  }
}

private func appRecorder(_ capture: VoiceCapture, ticks: Ticks, maxMs: UInt64 = 60_000) -> MediaVoiceRecorder {
  MediaVoiceRecorder(capture: capture, maxDurationMs: maxMs, clock: { ticks.read() }, levelInterval: .milliseconds(5))
}

extension GlobalMedia {
  @Suite(.serialized, .timeLimit(.minutes(2)))
  struct VoiceStateTests {
    // V1
    @Test
    func inv_voice_13_an_accepted_start_shows_a_recording_take() async throws {
      let app = try await AppLayer.open()
      let recorder = appRecorder(FakeCapture(), ticks: Ticks())
      #expect(recorder.state == VoiceState(take: 0, phase: .idle, outcome: nil, recordedMs: 0))
      let states = Collected(recorder.states)

      try await recorder.start(to: temporaryFile("m4a"))

      await eventually("recording") { states.last?.phase == .recording }
      #expect(Array(states.values.prefix(2)) == [VoiceState(take: 0, phase: .idle, outcome: nil, recordedMs: 0), VoiceState(take: 1, phase: .recording, outcome: nil, recordedMs: 0)])
      #expect(recorder.state == VoiceState(take: 1, phase: .recording, outcome: nil, recordedMs: 0))
      _ = try? await recorder.stop()
      await app.close()
    }

    // V2: the end by the layer close is in inv_ios_04
    @Test(arguments: [("interruption", VoiceRefusal.interrupted), ("call", .inCall), ("permission", .permissionDenied)])
    func inv_voice_14_a_revoked_take_ends_refused_without_stop_and_leaves_no_file(cause: String, reason: VoiceRefusal) async throws {
      let app = try await AppLayer.open()
      let recorder = appRecorder(FakeCapture(), ticks: Ticks())
      let states = Collected(recorder.states)
      let target = temporaryFile("m4a")
      try await recorder.start(to: target)
      await eventually("recording") { states.last?.phase == .recording }

      var call: MediaCallAudio?
      switch cause {
      case "interruption": app.session.send(.interruptionBegan)
      case "call": call = try await app.media.callAudio(.unmanaged)
      default: app.microphone.set(false)
      }

      await eventually("\(cause): the take ended by itself") { states.last?.phase == .idle && states.last?.outcome != nil }
      #expect(states.last == VoiceState(take: 1, phase: .idle, outcome: .refused(reason: reason), recordedMs: 0), "\(cause)")
      #expect(!FileManager.default.fileExists(atPath: target.path), "\(cause)")
      call?.end()
      await app.close()
    }

    // V3
    @Test
    func inv_voice_15_the_duration_limit_ends_the_take_recorded_without_stop() async throws {
      let app = try await AppLayer.open()
      let ticks = Ticks()
      let capture = FakeCapture()
      let recorder = appRecorder(capture, ticks: ticks, maxMs: 30_000)
      let states = Collected(recorder.states)
      let target = temporaryFile("m4a")
      try await recorder.start(to: target)

      ticks.advance(30_000)
      capture.limit?()

      await eventually("the take ended by the limit") { states.last?.outcome != nil }
      guard case let .recorded(file)? = states.last?.outcome else {
        Issue.record("not recorded: \(String(describing: states.last))")
        return
      }
      #expect((file.path, file.durationMs) == (target.path, 30_000))
      #expect(states.last?.phase == .idle && states.last?.take == 1)
      #expect(FileManager.default.fileExists(atPath: target.path))
      await app.close()
    }

    // V4
    @Test
    func inv_voice_16_a_recorder_failure_ends_the_take_failed_and_the_next_take_starts() async throws {
      let app = try await AppLayer.open()
      let capture = FakeCapture()
      let recorder = appRecorder(capture, ticks: Ticks())
      let states = Collected(recorder.states)
      let target = temporaryFile("m4a")
      try await recorder.start(to: target)
      await eventually("recording") { states.last?.phase == .recording }

      capture.failure?(NSError(domain: "com.apple.coreaudio.avfaudio", code: 561_017_449, userInfo: [NSLocalizedDescriptionKey: "encoder failed"]))

      await eventually("the take failed") { states.last?.outcome != nil }
      guard case let .failed(failure)? = states.last?.outcome else {
        Issue.record("not failed: \(String(describing: states.last))")
        return
      }
      #expect((failure.source, failure.domain, failure.code) == (.recorder, "com.apple.coreaudio.avfaudio", 561_017_449))
      #expect(states.last?.phase == .idle && states.last?.take == 1)
      #expect(!FileManager.default.fileExists(atPath: target.path))
      try await recorder.start(to: temporaryFile("m4a"))
      await eventually("the next take records") { states.last == VoiceState(take: 2, phase: .recording, outcome: nil, recordedMs: 0) }
      _ = try? await recorder.stop()
      await app.close()
    }

    // V5
    @Test(arguments: ["capture permission", "capture busy", "port permission", "call", "empty"])
    func inv_voice_17_every_refusal_carries_its_reason_in_the_error(cause: String) async throws {
      let app = try await AppLayer.open(microphone: SwitchMicrophone(granted: cause != "port permission"))
      let ticks = Ticks()
      let capture: FakeCapture
      let expected: VoiceRefusal
      switch cause {
      case "capture permission": capture = FakeCapture(refusal: .permissionDenied); expected = .permissionDenied
      case "capture busy": capture = FakeCapture(refusal: .microphoneBusy); expected = .microphoneBusy
      case "port permission": capture = FakeCapture(); expected = .permissionDenied
      case "call": capture = FakeCapture(); expected = .inCall
      default: capture = FakeCapture(writes: false); expected = .empty
      }
      let call = cause == "call" ? try await app.media.callAudio(.unmanaged) : nil
      let recorder = appRecorder(capture, ticks: ticks)

      do {
        try await recorder.start(to: temporaryFile("m4a"))
        ticks.advance(1000)
        _ = try await recorder.stop()
        Issue.record("\(cause): nothing was refused")
      } catch let MediaError.Voice(detail, reason) {
        #expect((detail, reason) == (voiceRefusalText(reason: expected), expected), "\(cause)")
      }
      #expect(recorder.state.outcome == .refused(reason: expected), "\(cause)")
      call?.end()
      await app.close()
    }

    // V6
    @Test(arguments: ["stop", "limit", "interruption"])
    func inv_voice_18_the_levels_end_with_every_take(end: String) async throws {
      let app = try await AppLayer.open()
      let ticks = Ticks()
      let capture = FakeCapture()
      let recorder = appRecorder(capture, ticks: ticks)
      let levels = recorder.levels
      try await recorder.start(to: temporaryFile("m4a"))
      ticks.advance(1000)

      switch end {
      case "stop": _ = try await recorder.stop()
      case "limit": capture.limit?()
      default: app.session.send(.interruptionBegan)
      }

      for await _ in levels {}
      #expect(recorder.state.phase == .idle, "\(end)")
      await app.close()
    }

    // D09
    @Test
    func inv_voice_19_a_call_applies_its_session_only_after_the_recorder_hardware_stopped() async throws {
      let app = try await AppLayer.open()
      let capture = StopWitness(session: app.session)
      let recorder = appRecorder(capture, ticks: Ticks())
      let states = Collected(recorder.states)
      try await recorder.start(to: temporaryFile("m4a"))
      await eventually("recording") { states.last?.phase == .recording }

      let call = try await app.media.callAudio(.unmanaged)

      await eventually("the call session is applied") { app.session.log.contains { $0.contains("VoiceChat") } }
      let atStop = try #require(capture.appliedAtStop, "the revoked take never stopped the hardware")
      #expect(!atStop.contains { $0.contains("VoiceChat") }, "the call session was applied before the recorder stopped: \(atStop)")
      call.end()
      await app.close()
    }

    // D25
    @Test
    func inv_voice_20_a_take_file_that_cannot_be_removed_is_one_file_failure_event() async throws {
      let app = try await AppLayer.open()
      let failures = Collected(Media.failures)
      let folder = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
      try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
      let recorder = appRecorder(StopWitness(session: app.session), ticks: Ticks())
      let states = Collected(recorder.states)
      try await recorder.start(to: folder.appendingPathComponent("take.m4a"))
      await eventually("recording") { states.last?.phase == .recording }
      try FileManager.default.setAttributes([.posixPermissions: 0o555], ofItemAtPath: folder.path)
      defer { try? FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: folder.path) }

      app.session.send(.interruptionBegan)

      await eventually("the refused take reported its file") { failures.values.contains { $0.source == .file } }
      #expect(failures.values.filter { $0.source == .file }.count == 1, "\(failures.values)")
      #expect(states.last == VoiceState(take: 1, phase: .idle, outcome: .refused(reason: .interrupted), recordedMs: 0))
      await app.close()
    }

    // A11
    @Test
    func inv_voice_21_a_cancelled_start_that_waits_for_admission_ends_and_the_next_take_is_admitted() async throws {
      let app = try await AppLayer.open()
      let gate = RecordingApplyGate()
      app.session.onApply = { gate.pass($0) }
      let recorder = appRecorder(FakeCapture(), ticks: Ticks())
      let outcome = StartOutcome()
      let start = Task {
        do { try await recorder.start(to: temporaryFile("m4a")); outcome.set(.success(())) }
        catch { outcome.set(.failure(error)) }
      }
      var reached = gate.reached.stream.makeAsyncIterator()
      await reached.next()

      start.cancel()

      await eventually("the cancelled start ended while the session apply still waits") { outcome.result != nil }
      gate.open()
      guard case let .failure(error)? = outcome.result, error is CancellationError else {
        Issue.record("the cancelled start did not end cancelled: \(String(describing: outcome.result))")
        await app.close()
        return
      }
      #expect(!recorder.isRecording)
      #expect(recorder.state == VoiceState(take: 0, phase: .idle, outcome: nil, recordedMs: 0))
      try await recorder.start(to: temporaryFile("m4a"))
      #expect(recorder.state == VoiceState(take: 1, phase: .recording, outcome: nil, recordedMs: 0))
      _ = try? await recorder.stop()
      await app.close()
    }

    // A14
    @Test(arguments: ["start", "stop"])
    func inv_failures_06_a_native_recorder_failure_reaches_media_failures_once_with_its_domain_and_code(step: String) async throws {
      let app = try await AppLayer.open()
      let native = NSError(domain: "com.apple.coreaudio.avfaudio", code: 1_718_449_215, userInfo: [NSLocalizedDescriptionKey: "\(step) failed"])
      let ticks = Ticks()
      let recorder = appRecorder(step == "start" ? FailingCapture(start: native) : FailingCapture(stop: native), ticks: ticks)
      let failures = Collected(Media.failures)

      do {
        try await recorder.start(to: temporaryFile("m4a"))
        ticks.advance(1000)
        _ = try await recorder.stop()
        Issue.record("\(step): nothing failed")
      } catch let error as NSError {
        #expect((error.domain, error.code) == (native.domain, native.code), "\(step): \(error)")
      }

      let barrier = NativeFailure(source: .file, domain: "barrier", code: 0, message: step)
      NativeFailures.publish(barrier)
      await eventually("\(step): every failure before the barrier arrived") { failures.values.contains(barrier) }
      let expected = NativeFailure(source: .recorder, domain: native.domain, code: Int64(native.code), message: native.localizedDescription)
      #expect(failures.values.filter { $0.source == .recorder } == [expected], "\(step): \(failures.values)")
      #expect(recorder.state.outcome == .failed(failure: expected), "\(step)")
      await app.close()
    }
  }
}
