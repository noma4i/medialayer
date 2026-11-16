@preconcurrency import AVFoundation
import Foundation
import MediaLayer
import MediaLayerTesting
import Testing
@testable import MediaLayerUI

final class FakeSessionPort: AudioSessionPort, @unchecked Sendable {
  private let lock = NSLock()
  private var entries: [String] = []
  private var busy: Int
  private var sink: (@Sendable (SystemAudioEvent) -> Void)?

  init(busyDeactivations: Int = 0) { busy = busyDeactivations }

  private var onApplyHook: (@Sendable (SessionPlan) -> Void)?
  var onApply: (@Sendable (SessionPlan) -> Void)? {
    get { lock.withLock { onApplyHook } }
    set { lock.withLock { onApplyHook = newValue } }
  }

  var log: [String] { lock.withLock { entries } }

  private func note(_ entry: String) {
    lock.withLock { entries.append(entry) }
    TestSignal.shared.fire()
  }

  func systemEvents(_ sink: @escaping @Sendable (SystemAudioEvent) -> Void) -> @Sendable () -> Void {
    lock.withLock { self.sink = sink }
    return { [weak self] in self?.lock.withLock { self?.sink = nil } }
  }

  func send(_ event: SystemAudioEvent) { lock.withLock { sink }?(event) }

  func apply(_ plan: SessionPlan, activate: Bool) throws {
    note("apply \(plan.category.rawValue) \(plan.mode.rawValue) activate=\(activate)")
    onApply?(plan)
  }

  func deactivate() throws {
    let refused = lock.withLock { () -> Bool in
      guard busy > 0 else { return false }
      busy -= 1
      return true
    }
    if refused {
      note("deactivate BUSY")
      throw AudioSessionBusy()
    }
    note("deactivate")
  }

  private var speakerFailure: Error?
  func failSpeaker(_ error: Error) { lock.withLock { speakerFailure = error } }
  func overrideSpeaker(_ on: Bool) throws {
    if let failure = lock.withLock({ speakerFailure }) { note("speaker failed"); throw failure }
    note("speaker \(on)")
  }
  func outputVolume() -> Float { 0.5 }
  func volumeChanges() -> AsyncStream<Float> { AsyncStream { _ in } }
}

final class FakeCallObserver: CallObserverPort, @unchecked Sendable {
  private let lock = NSLock()
  private var sink: (@Sendable (Set<String>) -> Void)?

  func calls(_ sink: @escaping @Sendable (Set<String>) -> Void) -> @Sendable () -> Void {
    lock.withLock { self.sink = sink }
    return { [weak self] in self?.lock.withLock { self?.sink = nil } }
  }

  func send(_ calls: Set<String>) { lock.withLock { sink }?(calls) }
}

struct GrantedMicrophone: MicrophonePermissionPort {
  func granted() -> Bool { true }
}

final class AudioRig: @unchecked Sendable {
  let core: MediaLayer
  let session: FakeSessionPort
  let calls = FakeCallObserver()
  let link: AudioLink
  private let lock = NSLock()
  private var seen: [MicrophoneAvailability] = []
  private var follower: Task<Void, Never>?

  init(session: FakeSessionPort = FakeSessionPort()) throws {
    self.session = session
    core = try MediaLayer.open(config: OpenConfig(
      dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
      fetcher: TestFetcher(), settings: QuietSettings(), slots: nil,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(), pool: defaultPoolSettings(), audioFocus: .exclusive
    ))
    link = AudioLink(core: core, ports: AudioPorts(session: session, calls: calls, permission: GrantedMicrophone()))
    let stream = link.availability
    follower = Task { [weak self] in
      for await value in stream {
        self?.lock.withLock { self?.seen.append(value) }
        TestSignal.shared.fire()
      }
    }
  }

  var availability: [MicrophoneAvailability] { lock.withLock { seen } }

  deinit {
    follower?.cancel()
    link.close()
    core.close()
  }
}

private func plan(_ category: AVAudioSession.Category, _ mode: AVAudioSession.Mode, _ options: AVAudioSession.CategoryOptions) -> SessionPlan {
  SessionPlan(category: category, mode: mode, options: options)
}

@Suite(.serialized, .timeLimit(.minutes(1)))
struct AudioLinkTests {
  @Test
  func inv_audio_34_every_wish_of_the_core_has_one_session_plan() {
    let table: [(SessionWant, SessionPlan)] = [
      (.playback(focus: .exclusive), plan(.playback, .moviePlayback, [])),
      (.playback(focus: .mix), plan(.playback, .moviePlayback, [.mixWithOthers])),
      (.playback(focus: .duck), plan(.playback, .moviePlayback, [.mixWithOthers, .duckOthers])),
      (.recording(kind: .voice), plan(.playAndRecord, .default, [.allowBluetoothA2DP])),
      (.recording(kind: .video), plan(.playAndRecord, .videoRecording, [.allowBluetoothA2DP])),
      (.call(kind: .unmanaged, speaker: false), plan(.playAndRecord, .voiceChat, [])),
      (.call(kind: .callKit(uuid: "a"), speaker: true), plan(.playAndRecord, .voiceChat, [])),
    ]
    for (want, expected) in table {
      #expect(SessionPlan.of(want) == expected, "\(want)")
    }
  }

  @Test
  func inv_audio_35_a_private_output_is_read_from_the_previous_route() {
    let table: [(AVAudioSession.Port, Bool)] = [
      (.headphones, true), (.bluetoothA2DP, true), (.bluetoothHFP, true), (.bluetoothLE, true), (.usbAudio, true),
      (.carAudio, true), (.airPlay, true), (.lineOut, true), (.HDMI, true),
      (.builtInSpeaker, false), (.builtInReceiver, false),
    ]
    for (port, private_) in table {
      #expect(hadPrivateOutput([port]) == private_, "\(port.rawValue)")
    }
    #expect(!hadPrivateOutput([]))
  }

  @Test
  func inv_audio_36_system_events_reach_the_core_and_its_actions_reach_the_session() async throws {
    let rig = try AudioRig()
    rig.link.audible(slot: 0, true)
    await eventually("playback applied") { rig.session.log == ["apply AVAudioSessionCategoryPlayback AVAudioSessionModeMoviePlayback activate=true"] }

    rig.session.send(.interruptionBegan)
    await eventually("the core heard the interruption") { rig.availability.last == .otherApp }
    rig.link.foreground(true)
    await eventually("the foreground left it") { rig.availability.last == .available }

    rig.session.send(.routeChanged(.oldDeviceUnavailable, hadPrivateOutput: true))
    rig.link.audible(slot: 0, false)
    await eventually("idle releases nothing more: the system already deactivated") { rig.availability.last == .available }
    #expect(Media.muted == false, "a lost output never writes the user flag")
  }

  @Test
  func inv_audio_37_a_busy_deactivation_is_finished_by_the_next_session_event() async throws {
    let rig = try AudioRig(session: FakeSessionPort(busyDeactivations: 1))
    rig.link.audible(slot: 0, true)
    rig.link.audible(slot: 0, false)
    await eventually("busy once") { rig.session.log.last == "deactivate BUSY" }

    rig.session.send(.routeChanged(.categoryChange, hadPrivateOutput: false))

    await eventually("released by the next event") { rig.session.log.last == "deactivate" }
  }

  @Test
  func inv_audio_38_a_callkit_call_only_configures_the_session_and_its_object_ends_the_call() async throws {
    let rig = try AudioRig()
    rig.link.audible(slot: 0, true)
    await eventually("playing") { rig.session.log.count == 1 }
    let clip = MediaItem(kind: .video, url: "https://o.test/v.mp4", headers: [:], cover: nil, blurHash: nil, strategy: .window)
    var call: MediaCallAudio? = try await rig.link.call(.callKit(UUID()))
    let states = StatesSeen(call!.states)
    await eventually("configured for CallKit") {
      Array(rig.session.log.suffix(2)) == ["deactivate", "apply AVAudioSessionCategoryPlayAndRecord AVAudioSessionModeVoiceChat activate=false"]
    }
    call?.systemActivated()
    await eventually("live") { states.values == [.connecting, .live] }
    await #expect(throws: MediaError.self) { try await rig.core.playUrl(source: clip) }

    call = nil

    await eventually("the released object ended the call") { states.values.last == .ended }
    _ = try await rig.core.playUrl(source: clip)
  }

  @Test
  func inv_audio_39_an_unmanaged_speaker_overrides_the_output_again_after_each_route_change() async throws {
    let rig = try AudioRig()
    let call = try await rig.link.call(.unmanaged)
    call.setSpeaker(true)
    await eventually("speaker") { rig.session.log.last == "speaker true" }

    rig.session.send(.routeChanged(.newDeviceAvailable, hadPrivateOutput: false))

    await eventually("applied again after the route") { rig.session.log.filter { $0 == "speaker true" }.count == 2 }
    call.end()
  }

  @Test
  func inv_audio_40_an_external_call_seen_by_the_observer_refuses_a_recording_until_it_ends() async throws {
    let rig = try AudioRig()
    rig.calls.send(["cellular"])
    await eventually("external") { rig.availability.last == .externalCall }
    switch await rig.link.admit(.voice) {
    case .success: Issue.record("a recording was admitted during an external call")
    case let .failure(error): #expect(error as? RecordingRefusal == .inCall)
    }

    rig.calls.send([])
    await eventually("free") { rig.availability.last == .available }

    guard case let .success(ticket) = await rig.link.admit(.voice) else {
      Issue.record("refused after the call")
      return
    }
    ticket.end()
  }

  @Test
  func inv_audio_41_an_interruption_ends_a_voice_recording_with_its_refusal_and_no_file() async throws {
    let rig = try AudioRig()
    let recorder = MediaVoiceRecorder(capture: WritingCapture(), maxDurationMs: 60_000, link: { rig.link }, clock: { 1000 }, levelInterval: .milliseconds(5))
    let target = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).m4a")
    try await recorder.start(to: target)

    rig.session.send(.interruptionBegan)

    await eventually("revoked") { !recorder.isRecording }
    do {
      _ = try await recorder.stop()
      Issue.record("a recording survived the interruption")
    } catch let MediaError.Voice(detail) {
      #expect(detail == voiceRefusalText(reason: .interrupted))
    }
    #expect(!FileManager.default.fileExists(atPath: target.path))
  }

  @Test
  func inv_audio_42_a_recording_without_an_open_layer_is_refused_and_records_nothing() async throws {
    let capture = WritingCapture()
    let recorder = MediaVoiceRecorder(capture: capture, maxDurationMs: 60_000, link: { nil }, clock: { 1000 }, levelInterval: .milliseconds(5))
    let target = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).m4a")
    await #expect(throws: MediaError.self) { try await recorder.start(to: target) }
    #expect(capture.starts == 0)
    #expect(!FileManager.default.fileExists(atPath: target.path))
  }

  @Test
  func inv_audio_51_a_call_that_arrives_with_the_admission_leaves_the_microphone_untouched() async throws {
    let rig = try AudioRig()
    let capture = WritingCapture()
    let recorder = MediaVoiceRecorder(capture: capture, maxDurationMs: 60_000, link: { rig.link }, clock: { 1000 }, levelInterval: .milliseconds(5))
    for round in 0..<20 {
      let calls = rig.calls
      rig.session.onApply = { plan in if plan.category == .playAndRecord { calls.send(["cellular"]) } }
      let target = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).m4a")
      do {
        try await recorder.start(to: target)
        Issue.record("round \(round): a recording started during the call")
        _ = try? await recorder.stop()
      } catch let MediaError.Voice(detail) {
        #expect(detail == voiceRefusalText(reason: .inCall), "round \(round)")
      }
      #expect(capture.starts == 0, "round \(round): the microphone was started during the call")
      #expect(!FileManager.default.fileExists(atPath: target.path))
      rig.session.onApply = nil
      rig.calls.send([])
      await eventually("free after round \(round)") { rig.availability.last == .available }
    }
  }

  @Test
  func inv_audio_53_closing_the_link_revokes_running_recordings_and_ends_calls() async throws {
    let rig = try AudioRig()
    guard case let .success(ticket) = await rig.link.admit(.voice) else { Issue.record("refused"); return }
    let revoked = Task { () -> RecordingRefusal? in
      for await reason in ticket.revoked { return reason }
      return nil
    }

    rig.link.close()

    #expect(await revoked.value == .interrupted)

    let callRig = try AudioRig()
    let call = try await callRig.link.call(.unmanaged)
    let states = Task { () -> [CallState] in
      var seen: [CallState] = []
      for await state in call.states { seen.append(state) }
      return seen
    }

    callRig.link.close()

    #expect(await states.value.last == .ended)
  }
  @Test
  func inv_audio_59_a_call_after_close_is_refused_and_a_failed_speaker_reaches_the_app() async throws {
    let closed = try AudioRig()
    closed.link.close()
    await #expect(throws: MediaError.self) { _ = try await closed.link.call(.unmanaged) }

    let rig = try AudioRig()
    let call = try await rig.link.call(.unmanaged)
    rig.session.failSpeaker(NSError(domain: "speaker", code: 7))
    call.setSpeaker(true)
    await eventually("the failure reached the app") { String(describing: Media.failure).contains("speaker") }
    call.end()
  }

}

final class StatesSeen: @unchecked Sendable {
  private let lock = NSLock()
  private var seen: [CallState] = []
  private var task: Task<Void, Never>?

  init(_ states: AsyncStream<CallState>) {
    task = Task { [weak self] in
      for await state in states {
        self?.lock.withLock { self?.seen.append(state) }
        TestSignal.shared.fire()
      }
    }
  }

  var values: [CallState] { lock.withLock { seen } }
  deinit { task?.cancel() }

}

final class WritingCapture: VoiceCapture, @unchecked Sendable {
  private let lock = NSLock()
  private var started = 0
  var starts: Int { lock.withLock { started } }

  func start(file: URL, maxDurationMs: UInt64, onLimit: @escaping @Sendable () -> Void) async throws {
    lock.withLock { started += 1 }
    try Data([1, 2, 3]).write(to: file)
  }

  func peak() -> Float { 0.5 }
  func stop() throws {}
}
