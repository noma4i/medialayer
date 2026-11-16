#if os(iOS)
@preconcurrency import AVFoundation
import Foundation
import MediaLayer
import Testing
@testable import MediaLayerUI

private final class SystemSession: AudioSessionPort, @unchecked Sendable {
  private static let quiet = SessionPlan(category: .soloAmbient, mode: .default, options: [])
  private let lock = NSLock()
  private var plan = SystemSession.quiet
  private var active = false
  private var interrupted = false
  private var changes = 0
  private var sinks: [AsyncStream<SystemAudioEvent>.Continuation] = []

  var state: (plan: SessionPlan, active: Bool, interrupted: Bool) { lock.withLock { (plan, active, interrupted) } }

  func setCategory(_ plan: SessionPlan) throws {
    lock.withLock { self.plan = plan }
    TestSignal.shared.fire()
  }

  func setActive(_ value: Bool) throws {
    try lock.withLock {
      if value && interrupted { throw NSError(domain: "system", code: 560_557_684) }
      active = value
    }
    TestSignal.shared.fire()
  }

  func currentPlan() -> SessionPlan { lock.withLock { plan } }

  var systemChanges: Int { lock.withLock { changes } }

  func systemEvents() -> AsyncStream<SystemAudioEvent> {
    let (stream, continuation) = AsyncStream<SystemAudioEvent>.makeStream()
    lock.withLock { sinks.append(continuation) }
    return stream
  }

  func outputVolume() -> Float { 0.5 }

  func volumeChanges() -> AsyncStream<Float> { AsyncStream { _ in } }

  func callKitActivates() { lock.withLock { active = true }; TestSignal.shared.fire() }

  func callKitDeactivates() { lock.withLock { active = false }; TestSignal.shared.fire() }

  func interruptionBegins() { change(.interruptionBegan) { active = false; interrupted = true } }

  func interruptionEnds() { change(.interruptionEnded) { interrupted = false } }

  func mediaServicesReset() { change(.mediaServicesReset) { active = false; plan = Self.quiet } }

  func routeChanges() { change(.routeChanged) {} }

  private func change(_ event: SystemAudioEvent, _ effect: () -> Void) {
    let targets = lock.withLock { () -> [AsyncStream<SystemAudioEvent>.Continuation] in
      effect()
      if case .routeChanged = event {} else { changes += 1 }
      return sinks
    }
    for sink in targets { sink.yield(event) }
    TestSignal.shared.fire()
  }
}

private final class SystemMicrophoneModel: MicrophonePort, @unchecked Sendable {
  private let lock = NSLock()
  private var state = MicrophoneState()
  private var sinks: [AsyncStream<String>.Continuation] = []

  func current() -> MicrophoneState { lock.withLock { state } }

  func changes() -> AsyncStream<String> {
    let (stream, continuation) = AsyncStream<String>.makeStream()
    lock.withLock { sinks.append(continuation) }
    return stream
  }

  var calls: Set<UUID> { lock.withLock { state.calls } }

  func permission(_ granted: Bool) { change("permission") { $0.permissionGranted = granted } }

  func callBegins(_ id: UUID) { change("call") { $0.calls.insert(id) } }

  func callEnds(_ id: UUID) { change("call") { $0.calls.remove(id) } }

  private func change(_ source: String, _ effect: (inout MicrophoneState) -> Void) {
    let targets = lock.withLock { () -> [AsyncStream<String>.Continuation] in
      effect(&state)
      return sinks
    }
    for sink in targets { sink.yield(source) }
    TestSignal.shared.fire()
  }
}

private final class Backend: CaptureBackend, @unchecked Sendable {
  let session = AVCaptureSession()
  let photoSize: PixelSize? = PixelSize(width: 3000, height: 4000)
  private let lock = NSLock()
  private var sink: (@Sendable (CaptureEvent) -> Void)?
  private let refuses: Bool

  init(refuses: Bool) {
    self.refuses = refuses
  }

  func setSink(_ sink: @escaping @Sendable (CaptureEvent) -> Void) { lock.withLock { self.sink = sink } }

  private func send(_ event: CaptureEvent) {
    lock.withLock { sink }?(event)
    TestSignal.shared.fire()
  }

  func bind(lens: CameraLens, profile: CaptureProfile) {
    guard !refuses else { return send(.refuse(reason: .permissionDenied)) }
    send(.opened(hasFlash: false, zoomMin: 1, zoomMax: 4))
    send(.streaming(on: true))
  }

  func unbind() { TestSignal.shared.fire() }
  func attachPreviewLayer(_ layer: AVCaptureVideoPreviewLayer) {}
  func detachPreviewLayer(_ layer: AVCaptureVideoPreviewLayer) {}
  func takePhoto(flash: Bool, profile: CaptureProfile) async throws -> Data { throw MediaError.Closed }

  func beginRecording(to url: URL, profile: CaptureProfile, lens: CameraLens) {
    try? Data([0, 0, 0, 1]).write(to: url)
    send(.recordStarted)
  }

  func endRecording() { send(.recordFinished(end: .completed)) }
  func recordedMs() -> UInt64 { 2000 }
  func setTorch(_ on: Bool) {}
  func setZoom(_ ratio: Float) {}
  func setFrameTap(_ tap: (@Sendable (CVPixelBuffer, UInt64) -> Void)?) {}
}

private final class Microphone: VoiceCapture, @unchecked Sendable {
  private let lock = NSLock()
  private var file: URL?
  private let refuses: Bool

  init(refuses: Bool) {
    self.refuses = refuses
  }

  func start(file: URL, maxDurationMs: UInt64, onLimit: @escaping @Sendable () -> Void) async throws {
    if refuses { throw VoiceRefused(reason: .permissionDenied) }
    lock.withLock { self.file = file }
  }

  func peak() -> Float { 0.5 }

  func stop() throws {
    if let file = lock.withLock({ file }) { try Data([1, 2, 3]).write(to: file) }
  }
}

private struct Seeded: RandomNumberGenerator {
  var state: UInt64

  mutating func next() -> UInt64 {
    state &+= 0x9E37_79B9_7F4A_7C15
    var mixed = state
    mixed = (mixed ^ (mixed >> 30)) &* 0xBF58_476D_1CE4_E5B9
    mixed = (mixed ^ (mixed >> 27)) &* 0x94D0_49BB_1331_11EB
    return mixed ^ (mixed >> 31)
  }
}

private enum Step: CaseIterable {
  case cameraShows, cameraHides, cameraRecords, cameraStops, cameraCancels, cameraDeniedShows, cameraDropped
  case appBackground, appForeground
  case voiceStarts, voiceStops, voiceStartsTwice, voiceStopsWhileStarting, voiceDenied
  case playerAudible, playerSilent
  case interruptionBegins, interruptionEnds, mediaServicesReset, routeChanges
  case callStarts, callSystemActivates, callSystemDeactivates, callSpeaker, callEnds
  case externalCallBegins, externalCallEnds
}

private final class Live: @unchecked Sendable {
  private let lock = NSLock()
  private var latest = false
  private var task: Task<Void, Never>?

  func follow(_ stream: AsyncStream<Bool>) {
    task = Task { [weak self] in
      for await value in stream {
        self?.lock.withLock { self?.latest = value }
        TestSignal.shared.fire()
      }
    }
  }

  var value: Bool { lock.withLock { latest } }

  deinit { task?.cancel() }
}

private final class World {
  let system = SystemSession()
  let microphone = SystemMicrophoneModel()
  var externalCall: UUID?
  var callUUID: UUID?
  let audio: MediaAudioSession
  let center = NotificationCenter()
  let directory = FileManager.default.temporaryDirectory.appendingPathComponent("audio-model-\(UUID().uuidString)")
  let signals = SystemSignalNames(
    memoryWarning: Notification.Name("audio.model.memory"),
    background: Notification.Name("audio.model.background"),
    foreground: Notification.Name("audio.model.foreground"),
    outputLost: Notification.Name("audio.model.output")
  )
  var camera: MediaCamera?
  var cameraShown = false
  var foreground = true
  var voice: MediaVoiceRecorder
  var audible: Set<UInt32> = []
  var call: CallAudioHold?
  var callManaged = false
  var callSpeaker = false
  var callKitLive = false
  var live = Live()
  var ticks: UInt64 = 0

  init() {
    audio = MediaAudioSession(focus: .mix, port: system, releaseDelay: .zero, microphonePort: microphone)
    voice = World.recorder(audio, refuses: false)
  }

  static func recorder(_ audio: MediaAudioSession, refuses: Bool) -> MediaVoiceRecorder {
    let clock = Clock()
    return MediaVoiceRecorder(capture: Microphone(refuses: refuses), maxDurationMs: 60_000, audio: { audio }, clock: { clock.tick() })
  }

  func makeCamera(refuses: Bool) throws -> MediaCamera {
    let audio = audio
    let services = CameraServices(
      admitSession: { _ in { } },
      admitShot: { _ in { } },
      audio: { audio },
      clipInfo: { _ in MediaInfo(durationMs: 2000, frame: ImageSize(width: 720, height: 1280)) },
      uprightVideo: { _ in },
      upright: { url, _ in url },
      directory: directory,
      center: center,
      signals: signals,
      clock: { 0 }
    )
    return try MediaCamera(profile: .circle, lens: .front, backend: Backend(refuses: refuses), services: services)
  }

  var cameraHoldsMicrophone: Bool {
    guard let camera, cameraShown else { return false }
    return [.ready, .recording, .stopping].contains(camera.state.phase)
  }

  var wantsMicrophone: Bool { voice.isRecording || cameraHoldsMicrophone }

  func callConverged(_ call: CallAudioHold) -> Bool {
    let now = system.state
    guard now.plan == .call(speaker: callSpeaker) else { return false }
    if callManaged { return live.value == (callKitLive && !now.interrupted) }
    if now.interrupted { return !live.value }
    return now.active && live.value
  }

  var externalCallLive: Bool { externalCall != nil && externalCall != callUUID }

  func microphoneConverged() -> Bool {
    let value = audio.microphone.current
    if call != nil { return value == .ownCall }
    if externalCallLive { return value == .externalCall }
    if system.state.interrupted { return value == .available || value == .otherApp }
    return value == .available
  }

  func converged() -> Bool {
    guard microphoneConverged() else { return false }
    if cameraShown && foreground && camera?.state.phase == .opening { return false }
    guard audio.isRecording == wantsMicrophone else { return false }

    if let call { return callConverged(call) }
    let now = system.state
    if now.interrupted { return true }
    if wantsMicrophone { return now.active && now.plan == .recording }
    if !audible.isEmpty { return now.active && now.plan == .playback(.mix) }
    return !now.active
  }

  func run(_ step: Step, _ random: inout Seeded) async throws {
    switch step {
    case .cameraShows:
      if camera == nil {
        camera = try makeCamera(refuses: false)
      }
      camera?.attachPreview()
      cameraShown = true
    case .cameraHides:
      camera?.detachPreview()
      cameraShown = false
    case .cameraRecords:
      try? await camera?.startRecording()
    case .cameraStops:
      _ = try? await camera?.stopRecording()
    case .cameraCancels:
      camera?.cancelRecording()
    case .cameraDeniedShows:
      camera?.detachPreview()
      camera = try makeCamera(refuses: true)
      camera?.attachPreview()
      cameraShown = true
    case .cameraDropped:
      camera = nil
      cameraShown = false
    case .appBackground:
      foreground = false
      center.post(name: signals.background, object: nil)
    case .appForeground:
      foreground = true
      center.post(name: signals.foreground, object: nil)
    case .voiceStarts:
      try? await voice.start(to: file())
    case .voiceStops:
      _ = try? await voice.stop()
    case .voiceStartsTwice:
      let recorder = voice
      let (one, two) = (file(), file())
      async let first: Void? = try? recorder.start(to: one)
      async let second: Void? = try? recorder.start(to: two)
      _ = await (first, second)
    case .voiceStopsWhileStarting:
      let recorder = voice
      let target = file()
      let starting = Task { try? await recorder.start(to: target) }
      _ = try? await recorder.stop()
      _ = await starting.value
    case .voiceDenied:
      if !voice.isRecording {
        microphone.permission(false)
        let refused = World.recorder(audio, refuses: false)
        try? await refused.start(to: file())
        microphone.permission(true)
        guard !refused.isRecording else { throw MediaError.Unexpected(detail: "a recording started without the microphone permission") }
      }
    case .playerAudible:
      let slot = UInt32.random(in: 1...3, using: &random)
      audible.insert(slot)
      audio.setAudible(slot: slot, true)
    case .playerSilent:
      let slot = UInt32.random(in: 1...3, using: &random)
      audible.remove(slot)
      audio.setAudible(slot: slot, false)
    case .interruptionBegins:
      system.interruptionBegins()
      callKitLive = false
    case .interruptionEnds:
      system.interruptionEnds()
    case .mediaServicesReset:
      system.mediaServicesReset()
    case .routeChanges:
      system.routeChanges()
    case .externalCallBegins:
      guard externalCall == nil else { return }
      let id = UUID()
      externalCall = id
      microphone.callBegins(id)
      if !voice.isRecording {
        let refused = World.recorder(audio, refuses: false)
        try? await refused.start(to: file())
        guard !refused.isRecording else { throw MediaError.Unexpected(detail: "a recording started during an external call") }
      }
    case .externalCallEnds:
      guard let id = externalCall else { return }
      externalCall = nil
      microphone.callEnds(id)
    case .callStarts:
      guard call == nil else { return }
      let managed = Bool.random(using: &random)
      let uuid: UUID? = managed ? UUID() : nil
      if let hold = await audio.holdCall(managedBySystem: managed, callUUID: uuid) {
        callUUID = uuid
        call = hold
        callManaged = managed
        callSpeaker = false
        callKitLive = false
        live = Live()
        live.follow(hold.liveChanges)
      }
    case .callSystemActivates:
      guard let call, callManaged, !system.state.interrupted else { return }
      system.callKitActivates()
      callKitLive = true
      call.systemActivated()
    case .callSystemDeactivates:
      guard let call, callManaged else { return }
      system.callKitDeactivates()
      callKitLive = false
      call.systemDeactivated()
    case .callSpeaker:
      guard let call else { return }
      callSpeaker.toggle()
      call.setSpeaker(callSpeaker)
    case .callEnds:
      guard let ending = call else { return }
      call = nil
      callUUID = nil
      ending.release()
      if callManaged && callKitLive {
        callKitLive = false
        system.callKitDeactivates()
      }
    }
  }

  private func file() -> URL {
    try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    return directory.appendingPathComponent("voice-\(UUID().uuidString).m4a")
  }
}

private final class Clock: @unchecked Sendable {
  private let lock = NSLock()
  private var now: UInt64 = 0

  func tick() -> UInt64 { lock.withLock { now += 500; return now } }
}

@Suite(.serialized, .timeLimit(.minutes(2)))
struct MediaAudioModelTests {
  @Test
  func inv_audio_12_anySequenceOfRecordersPlayersAndSystemEventsLeavesTheSessionMatchingTheDemand() async throws {
    let seed = ProcessInfo.processInfo.environment["ML_SEED"].flatMap(UInt64.init) ?? UInt64.random(in: 1...UInt64.max)
    print("repeat: ML_SEED=\(seed)")
    var random = Seeded(state: seed)
    let world = World()
    var trail: [Step] = []
    for _ in 0..<200 {
      let step = Step.allCases[Int.random(in: 0..<Step.allCases.count, using: &random)]
      trail.append(step)
      try await world.run(step, &random)
      await eventually("session matches the demand after \(trail.suffix(8)) (ML_SEED=\(seed))") { world.converged() }
      if !world.converged() { return }
    }
    world.system.interruptionEnds()
    if let id = world.externalCall { world.microphone.callEnds(id) }
    world.externalCall = nil
    world.call?.release()
    world.call = nil
    if world.callKitLive { world.system.callKitDeactivates() }
    world.camera = nil
    _ = try? await world.voice.stop()
    for slot in world.audible { world.audio.setAudible(slot: slot, false) }
    world.audible = []
    await eventually("the session is given back at the end (ML_SEED=\(seed))") { world.converged() && !world.system.state.active }
  }
}
#endif
