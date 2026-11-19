#if os(iOS)
import AVFoundation
import Foundation
import MediaLayer
import SwiftUI
import Testing
import UIKit
@testable import MediaLayerUI

private struct DeniedMicrophone: MicrophonePermissionPort {
  func state() -> MicrophonePermission { .denied }
}

private final class FakeBackend: CaptureBackend, CameraMicrophone, @unchecked Sendable {
  enum Answer { case opens, refuses(CameraRefusal), silent }

  let session = AVCaptureSession()
  private let lock = NSLock()
  private var sink: (@Sendable (CaptureEvent) -> Void)?
  private(set) var log: [String] = []
  private(set) var mainThreadCalls = 0
  private(set) var recordingURL: URL?
  private(set) var tap: (@Sendable (CVPixelBuffer, UInt64) -> Void)?
  private(set) var recordedReads = 0
  let readEvents = AsyncStream<Void>.makeStream()
  private(set) var recordedProfile: CaptureProfile?
  var answer = Answer.opens
  var hasFlash = true
  var photoFails = false
  /// The preview microphone sensor hears sound once bound, as AVCaptureBackend reports from audio buffers; a test sends silence itself.
  var soundOnBind = true
  /// The first PCM of a take, as the relay reports it to the core before the take starts; a test of a silent source turns it off.
  var soundOnTake = true
  /// A real file the take writes instead of three bytes.
  var clipFile: URL?
  var photoSize: PixelSize? { PixelSize(width: 3000, height: 4000) }
  /// The verdict the writer of a take gives with its outcome; nil is a backend without a ledger.
  var coverage: AudioCoverage? = AudioCoverage(verdict: .covered, samples: 0, firstUs: nil, lastUs: nil, gapAtUs: nil, gapUs: nil)
  func takeCoverage() -> AudioCoverage? { lock.withLock { coverage } }
  func setSink(_ sink: @escaping @Sendable (CaptureEvent) -> Void) { lock.withLock { self.sink = sink } }

  func emit(_ event: CaptureEvent) { lock.withLock { sink }?(event) }

  func count(_ entry: String) -> Int { lock.withLock { log.filter { $0 == entry }.count } }

  private func note(_ entry: String) {
    lock.withLock {
      log.append(entry)
      if Thread.isMainThread { mainThreadCalls += 1 }
    }
    TestSignal.shared.fire()
  }

  func bind(lens: CameraLens, profile: CaptureProfile) {
    note("bind \(lens)")
    switch answer {
    case .opens:
      emit(.opened(hasFlash: hasFlash, zoomMin: 1, zoomMax: 20))
      emit(.streaming(on: true))
      if profile.audio && lock.withLock({ soundOnBind }) { emit(.microphoneFlow(true)) }
    case let .refuses(reason): emit(.refuse(reason: reason))
    case .silent: break
    }
  }

  func unbind() { note("unbind") }

  /// The hardware stop of a real session ends later on its own queue: a test holds it and ends it itself.
  var deferUnbind = false
  private var heldUnbinds: [@Sendable () -> Void] = []
  func unbind(completion: @escaping @Sendable () -> Void) {
    unbind()
    if lock.withLock({ deferUnbind }) { lock.withLock { heldUnbinds.append(completion) } } else { completion() }
  }
  func finishUnbind() {
    let held = lock.withLock { let all = heldUnbinds; heldUnbinds = []; return all }
    held.forEach { $0() }
  }

  /// A native microphone: the camera opens its audio input only after the recording is admitted.
  var nativeMicrophone = false
  var audio: CaptureAudio { nativeMicrophone ? .native(self) : .managed }
  private var microphoneListener: (UInt64, CameraMicrophoneListener)?
  func start(generation: UInt64, listener: CameraMicrophoneListener) {
    lock.withLock { microphoneListener = (generation, listener) }
    note("mic start")
  }
  /// The native microphone of the current generation hears sound or loses it.
  func micHeard() { if let (generation, listener) = lock.withLock({ microphoneListener }) { listener.microphoneHeard(generation: generation) } }
  func micLost() { if let (generation, listener) = lock.withLock({ microphoneListener }) { listener.microphoneLost(generation: generation, error: nil) } }
  func stop(generation: UInt64) { note("mic stop") }

  /// The hardware does not report the start of a take until the test does.
  var holdRecordStart = false
  func attachPreviewLayer(_ layer: AVCaptureVideoPreviewLayer) {
    note("attachPreview")
    layer.setSessionWithNoConnection(session)
  }
  func detachPreviewLayer(_ layer: AVCaptureVideoPreviewLayer) { note("detachPreview") }

  func takePhoto(flash: Bool, profile: CaptureProfile) async throws -> Data {
    note("photo flash=\(flash)")
    if photoFails { throw MediaError.Camera(reason: .photoFailed, detail: "fake", recordedMs: nil) }
    return Data([0xFF, 0xD8, 0xFF])
  }

  func beginRecording(to url: URL, profile: CaptureProfile, lens: CameraLens) {
    note("record \(lens)")
    lock.withLock { recordingURL = url; recordedProfile = profile }
    if let clipFile { try? FileManager.default.copyItem(at: clipFile, to: url) } else { try? Data([1, 2, 3]).write(to: url) }
    if holdRecordStart { return }
    if profile.audio && soundOnTake { emit(.microphoneFlow(true)) }
    emit(.recordStarted)
  }

  func endRecording() {
    note("endRecording")
    emit(.recordFinished(end: .completed))
  }

  func recordedMs() -> UInt64 {
    lock.withLock { recordedReads += 1 }
    TestSignal.shared.fire()
    readEvents.continuation.yield(())
    return 700
  }

  var commandFailure: NSError?
  func setTorch(_ on: Bool) throws {
    note("torch \(on)")
    if let commandFailure { throw commandFailure }
  }
  func setZoom(_ ratio: Float) throws {
    note("zoom \(ratio)")
    if let commandFailure { throw commandFailure }
  }
  func setFrameTap(_ tap: (@Sendable (CVPixelBuffer, UInt64) -> Void)?) { lock.withLock { self.tap = tap }; TestSignal.shared.fire() }
}

/// The voice take on its own microphone: it starts once the core connects its input and the microphone hears sound.
private final class InputVoiceCapture: VoiceCapture, CameraMicrophone, @unchecked Sendable {
  private let link: @Sendable () -> AudioLink?
  private let lock = NSLock()
  private var input: CameraAudioInput?
  private(set) var log: [String] = []
  private var heard: CheckedContinuation<Void, Never>?
  init(link: @escaping @Sendable () -> AudioLink?) { self.link = link }

  func start(file: URL, maxDurationMs: UInt64, onLimit: @escaping @Sendable () -> Void, onFailure: @escaping @Sendable (Error) -> Void) async throws {
    guard let link = link() else { throw MediaError.Closed }
    await withCheckedContinuation { continuation in
      lock.withLock { heard = continuation }
      let made = CameraAudioInput(link: link, microphone: self, kind: .voice, failureSource: nil) { _ in }
      lock.withLock { input = made }
    }
    try Data([1, 2, 3]).write(to: file)
  }
  func start(generation: UInt64, listener: CameraMicrophoneListener) {
    let waiting = lock.withLock { log.append("voice start"); defer { heard = nil }; return heard }
    DispatchQueue.global().async { listener.microphoneHeard(generation: generation); waiting?.resume() }
  }
  func stop(generation: UInt64) { lock.withLock { log.append("voice stop") } }
  func peak() -> Float { 0 }
  func stop() throws { lock.withLock { let made = input; input = nil; return made }?.close() }
}

private final class Counters: @unchecked Sendable {
  private let lock = NSLock()
  private var taken = 0
  private var given = 0
  private var uprightQualities: [Double] = []
  private var mainCalls = 0
  var info: MediaInfo? = MediaInfo(durationMs: 2000, frame: ImageSize(width: 720, height: 1280))
  /// An error the reservation of a session or a shot throws; nil lets it pass.
  var sessionError: Error?
  var shotError: Error?

  var sessionsTaken: Int { lock.withLock { taken } }
  var released: Int { lock.withLock { given } }
  var qualities: [Double] { lock.withLock { uprightQualities } }
  var mainThread: Int { lock.withLock { mainCalls } }

  func took() { lock.withLock { taken += 1 }; TestSignal.shared.fire() }
  func gave() { lock.withLock { given += 1 }; TestSignal.shared.fire() }
  func upright(_ quality: Double) { lock.withLock { uprightQualities.append(quality) }; TestSignal.shared.fire() }
  func checkThread() { if Thread.isMainThread { lock.withLock { mainCalls += 1 } } }
  private var rescannedURLs: [URL] = []
  var rescanned: [URL] { lock.withLock { rescannedURLs } }
  func rescan(_ url: URL) -> AudioCoverage { lock.withLock { rescannedURLs.append(url) }; return takeCoverageOf(.covered) }
}

private final class StateObservation {
  let task: Task<Void, Never>
  init(_ states: AsyncStream<CameraState>) {
    task = Task { for await _ in states { TestSignal.shared.fire() } }
  }
  deinit { task.cancel() }
}

private func takeCoverageOf(_ verdict: CoverageVerdict) -> AudioCoverage {
  AudioCoverage(verdict: verdict, samples: 0, firstUs: nil, lastUs: nil, gapAtUs: nil, gapUs: nil)
}

@MainActor
private final class LayerLog {
  var count = 0
}

@MainActor
private final class PhaseLog {
  var last: CameraPhase?
}

@MainActor
private final class CameraSwap: ObservableObject {
  @Published var camera: MediaCamera
  init(camera: MediaCamera) { self.camera = camera }
}

private struct SwapCell: View {
  @ObservedObject var model: CameraSwap
  let layers: LayerLog
  var body: some View {
    MediaCameraPreview(camera: model.camera) { _ in layers.count += 1 }
  }
}

private struct StateCell: View {
  @ObservedObject var model: CameraSwap
  let seen: PhaseLog
  var body: some View {
    WithMediaCameraState(camera: model.camera) { state in
      Color.clear.onAppear { seen.last = state.phase }.onChange(of: state.phase) { seen.last = $0 }
    }
  }
}

private struct Rig {
  let camera: MediaCamera
  let backend: FakeBackend
  let counters: Counters
  let center: NotificationCenter
  let directory: URL
  let audio: TestAudioRig
  let signals: SystemSignalNames
  let observation: StateObservation
}

private let names = SystemSignalNames(
  memoryWarning: Notification.Name("camera.test.memory"),
  background: Notification.Name("camera.test.background"),
  foreground: Notification.Name("camera.test.foreground")
)

private func jpeg() -> Data {
  let format = UIGraphicsImageRendererFormat()
  format.scale = 1
  return UIGraphicsImageRenderer(size: CGSize(width: 3, height: 5), format: format).jpegData(withCompressionQuality: 0.9) { context in
    UIColor.red.setFill()
    context.fill(CGRect(x: 0, y: 0, width: 3, height: 5))
  }
}

private func makeRig(_ profile: CaptureProfile = .photo, lens: CameraLens = .back, screen: ScreenSwitch = ScreenSwitch(), permission: MicrophonePermissionPort = GrantedMicrophone(), retry: RetryTimer = .system, configure: (FakeBackend, Counters) -> Void = { _, _ in }) throws -> Rig {
  let backend = FakeBackend()
  let counters = Counters()
  configure(backend, counters)
  let center = NotificationCenter()
  let directory = FileManager.default.temporaryDirectory.appendingPathComponent("camera-\(UUID().uuidString)")
  let audio = try TestAudioRig(permission: permission)
  let image = jpeg()
  let services = CameraServices(
    admitSession: { _ in
      counters.checkThread()
      if let error = counters.sessionError { throw error }
      counters.took()
      return { counters.gave() }
    },
    admitShot: { _ in
      counters.checkThread()
      if let error = counters.shotError { throw error }
      counters.took()
      return { counters.gave() }
    },
    link: { audio.link },
    clipInfo: { url in
      counters.checkThread()
      return counters.info
    },
    upright: { raw, quality in
      counters.checkThread()
      counters.upright(quality)
      let made = raw.deletingLastPathComponent().appendingPathComponent("\(UUID().uuidString).jpg")
      try image.write(to: made)
      return made
    },
    audioCoverage: { url, _ in counters.rescan(url) },
    directory: directory,
    center: center,
    signals: names,
    clock: { 0 }
  )
  let camera = try MediaCamera(profile: profile, lens: lens, backend: backend, services: services, screen: screen.awake(), retry: retry)
  let observation = StateObservation(camera.states)
  return Rig(camera: camera, backend: backend, counters: counters, center: center, directory: directory, audio: audio, signals: names, observation: observation)
}

private func ready(_ rig: Rig) async {
  rig.camera.attachPreview()
  await eventually("ready") { rig.camera.state.phase == .ready }
}

private func files(_ rig: Rig) -> [String] {
  ((try? FileManager.default.contentsOfDirectory(atPath: rig.directory.path)) ?? []).sorted()
}

@Suite(.serialized, .timeLimit(.minutes(1)))
struct MediaCameraTests {
  @Test
  func inv_audio_85_a_voice_right_after_a_circle_stop_takes_the_microphone_of_the_live_preview_and_is_busy_during_a_take() async throws {
    let rig = try makeRig(.moment) { backend, _ in backend.nativeMicrophone = true }
    await ready(rig)
    await eventually("the preview holds the microphone") { rig.backend.count("mic start") == 1 }
    try await rig.camera.startRecording()
    let during = MediaVoiceRecorder(capture: InputVoiceCapture(link: { rig.audio.link }), maxDurationMs: 60_000, link: { rig.audio.link })
    do {
      try await during.start(to: FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).m4a"))
      Issue.record("a voice during a camera take started")
    } catch let MediaError.Voice(_, reason) {
      #expect(reason == .microphoneBusy)
    }
    #expect(rig.backend.count("mic stop") == 0, "the take keeps its microphone")
    _ = try await rig.camera.stopRecording()

    let capture = InputVoiceCapture(link: { rig.audio.link })
    let voice = MediaVoiceRecorder(capture: capture, maxDurationMs: 60_000, link: { rig.audio.link })
    try await voice.start(to: FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).m4a"))
    #expect(rig.backend.count("mic stop") == 1, "the preview gave its microphone to the voice")
    #expect(capture.log == ["voice start"])
    _ = try? await voice.stop()
    await eventually("the preview took its microphone back") { rig.backend.count("mic start") == 2 }
  }

  /// A camera whose native microphone is silent until the test says so, on the manual clock of the attempts.
  private func silentMicrophoneRig(_ retry: ManualTimer) async throws -> Rig {
    let rig = try makeRig(.moment, retry: retry.timer) { backend, _ in
      backend.nativeMicrophone = true
      backend.soundOnBind = false
      backend.soundOnTake = false
    }
    await ready(rig)
    await eventually("the preview holds the microphone") { rig.backend.count("mic start") == 1 }
    return rig
  }

  private func loseAttempt(_ rig: Rig, _ retry: ManualTimer, _ attempt: Int) async {
    await eventually("attempt \(attempt)") { rig.backend.count("mic start") == attempt }
    rig.backend.micLost()
    await eventually("the pause after attempt \(attempt)") { retry.armed(MicrophoneRetry.pauseMs) }
    retry.fire(MicrophoneRetry.pauseMs)
  }

  @Test
  func inv_audio_115_a_camera_take_whose_microphone_is_lost_twice_starts_on_the_third_attempt_as_one_take() async throws {
    let retry = ManualTimer()
    let rig = try await silentMicrophoneRig(retry)
    let start = Task { try await rig.camera.startRecording() }
    await eventually("the take began") { rig.backend.count("record back") == 1 }
    await loseAttempt(rig, retry, 1)
    await loseAttempt(rig, retry, 2)
    await eventually("attempt 3") { rig.backend.count("mic start") == 3 }
    rig.backend.micHeard()
    rig.backend.emit(.microphoneFlow(true))
    try await start.value
    #expect(rig.camera.state.phase == .recording)
    #expect(rig.backend.count("record back") == 1, "one take")
    _ = try await rig.camera.stopRecording()
  }

  @Test
  func inv_audio_116_a_camera_take_whose_microphone_is_lost_on_all_three_attempts_is_microphone_silent() async throws {
    let retry = ManualTimer()
    let rig = try await silentMicrophoneRig(retry)
    let start = Task { try await rig.camera.startRecording() }
    await eventually("the take began") { rig.backend.count("record back") == 1 }
    await loseAttempt(rig, retry, 1)
    await loseAttempt(rig, retry, 2)
    await eventually("attempt 3") { rig.backend.count("mic start") == 3 }
    rig.backend.micLost()
    do {
      try await start.value
      Issue.record("a take without sound started")
    } catch let MediaError.Camera(reason, _, _) {
      #expect(reason == .microphoneSilent)
    }
    #expect(rig.backend.count("mic start") == 3, "exactly three sources")
    #expect(!retry.armed(MicrophoneRetry.pauseMs))
    await eventually("ready after the silent take") { rig.camera.state.phase == .ready }
    let again = Task { try await rig.camera.startRecording() }
    await eventually("the next start runs the attempts again") { rig.backend.count("mic start") == 4 }
    rig.backend.micHeard()
    rig.backend.emit(.microphoneFlow(true))
    try await again.value
    #expect(rig.camera.state.phase == .recording, "the sound of the retry starts the take")
    _ = try await rig.camera.stopRecording()
  }

  @Test
  func inv_audio_87_a_recording_of_another_app_never_blocks_a_voice_start() async throws {
    let rig = try makeRig(.moment)
    let seen = Collected(rig.audio.link.availability)
    rig.audio.link.send(.otherRecording(active: true))
    await eventually("another app records") { seen.values.last == .otherApp }
    let capture = InputVoiceCapture(link: { rig.audio.link })
    let voice = MediaVoiceRecorder(capture: capture, maxDurationMs: 60_000, link: { rig.audio.link })
    try await voice.start(to: FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).m4a"))
    #expect(capture.log == ["voice start"], "the voice took the microphone next to another app")
    _ = try? await voice.stop()
  }

  @Test
  func inv_audio_86_a_voice_right_after_the_camera_closed_starts_without_a_blink_of_no_sound() async throws {
    let rig = try makeRig(.moment) { backend, _ in backend.nativeMicrophone = true }
    await ready(rig)
    await eventually("the preview holds the microphone") { rig.backend.count("mic start") == 1 }
    rig.camera.detachPreview()
    await eventually("the camera closed") { rig.backend.count("mic stop") == 1 && !rig.audio.isRecording }

    let seen = Collected(rig.audio.link.availability)
    let capture = InputVoiceCapture(link: { rig.audio.link })
    let voice = MediaVoiceRecorder(capture: capture, maxDurationMs: 60_000, link: { rig.audio.link })
    try await voice.start(to: FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).m4a"))
    _ = await rig.audio.link.readVolume()
    #expect(!seen.values.contains(.noSound), "the start blinked no sound: \(seen.values)")
    _ = try? await voice.stop()
  }

  @Test
  func inv_capture_13_the_session_takes_its_memory_before_the_bind_and_gives_it_back_when_the_preview_leaves() async throws {
    let rig = try makeRig()
    await ready(rig)
    #expect((rig.counters.sessionsTaken, rig.counters.released) == (1, 0))
    rig.camera.detachPreview()
    await eventually("released") { rig.counters.released == 1 }
    #expect(rig.backend.count("unbind") == 1)
  }

  @Test
  func inv_capture_13_a_refusal_of_the_memory_budget_fails_the_camera_without_a_bind() async throws {
    let rig = try makeRig { _, counters in counters.sessionError = MediaError.Overloaded(detail: "no memory") }
    rig.camera.attachPreview()
    await eventually("failed") { rig.camera.state.phase == .failed }
    #expect(rig.camera.state.fault == .memory)
    #expect(rig.backend.log.filter { $0.hasPrefix("bind") }.isEmpty)
  }

  @Test(arguments: [
    (error: MediaError.Closed, reason: CameraRefusal.notReady),
    (error: MediaError.Overloaded(detail: "queue"), reason: CameraRefusal.memory),
    (error: MediaError.Unexpected(detail: "native"), reason: CameraRefusal.notReady),
  ])
  func inv_capture_13_a_refused_session_reservation_tells_its_cause(error: MediaError, reason: CameraRefusal) async throws {
    let rig = try makeRig { _, counters in counters.sessionError = error }
    rig.camera.attachPreview()
    await eventually("failed") { rig.camera.state.phase == .failed }
    #expect(rig.camera.state.fault == reason)
  }

  @Test(arguments: [
    (error: MediaError.Closed, reason: CameraRefusal.notReady),
    (error: MediaError.Overloaded(detail: "queue"), reason: CameraRefusal.memory),
    (error: MediaError.Unexpected(detail: "native"), reason: CameraRefusal.notReady),
  ])
  func inv_capture_13_a_refused_shot_reservation_tells_its_cause(error: MediaError, reason: CameraRefusal) async throws {
    let rig = try makeRig()
    await ready(rig)
    rig.counters.shotError = error
    do {
      _ = try await rig.camera.takePhoto()
      Issue.record("a refused shot was taken")
    } catch let MediaError.Camera(got, _, _) {
      #expect(got == reason)
    }
  }

  @Test
  func inv_capture_13_the_memory_of_a_shot_is_given_back_after_a_photo_and_after_a_failed_photo() async throws {
    let rig = try makeRig()
    await ready(rig)
    _ = try await rig.camera.takePhoto()
    #expect((rig.counters.sessionsTaken, rig.counters.released) == (2, 1))
    rig.backend.photoFails = true
    await #expect(throws: MediaError.self) { _ = try await rig.camera.takePhoto() }
    #expect((rig.counters.sessionsTaken, rig.counters.released) == (3, 2))
    #expect(rig.camera.state.fault == .photoFailed)
    rig.counters.shotError = MediaError.Overloaded(detail: "no memory")
    do {
      _ = try await rig.camera.takePhoto()
      Issue.record("a shot above the budget was taken")
    } catch let MediaError.Camera(reason, _, _) {
      #expect(reason == .memory)
    }
  }

  @Test
  func inv_capture_15_a_photo_is_an_upright_jpeg_file_with_its_size_and_the_lens_and_the_raw_file_is_gone() async throws {
    let rig = try makeRig(.photo, lens: .front)
    await ready(rig)
    let photo = try await rig.camera.takePhoto()
    #expect((photo.width, photo.height, photo.lens) == (3, 5, .front))
    #expect(photo.url.pathExtension == "jpg" && photo.url.lastPathComponent.hasPrefix("capture-"))
    #expect(FileManager.default.fileExists(atPath: photo.url.path))
    #expect(rig.counters.qualities == [0.9])
    #expect(files(rig) == [photo.url.lastPathComponent], "no raw file stays in the capture directory")
    #expect(rig.camera.state.shooting == false)
  }

  @Test
  func inv_api_63_a_camera_preview_on_screen_keeps_the_screen_on() async throws {
    let screen = ScreenSwitch()
    let rig = try makeRig(.moment, screen: screen)
    #expect(!screen.on)

    await ready(rig)
    await eventually("the screen stays on while the capture screen shows the preview") { screen.on }

    try await rig.camera.startRecording()
    _ = try await rig.camera.stopRecording()
    #expect(screen.on, "a recording keeps the hold of its preview")

    rig.camera.detachPreview()
    await eventually("the screen may sleep after the preview leaves") { !screen.on }
  }

  @Test
  func inv_capture_14_a_thrown_away_short_and_failed_recording_leaves_no_file_and_a_delivered_one_stays() async throws {
    let rig = try makeRig(.moment)
    await ready(rig)
    try await rig.camera.startRecording()
    let delivered = try await rig.camera.stopRecording()
    #expect((delivered.durationMs, delivered.frame?.width, delivered.lens) == (2000, 720, .back))
    #expect(files(rig) == [delivered.url.lastPathComponent])
    #expect(rig.camera.state.phase == .ready)

    rig.counters.info = MediaInfo(durationMs: 300, frame: nil)
    try await rig.camera.startRecording()
    do {
      _ = try await rig.camera.stopRecording()
      Issue.record("a recording of 300 ms was delivered")
    } catch let MediaError.Camera(reason, detail, _) {
      #expect((reason, detail) == (.tooShort, cameraRefusalText(reason: .tooShort)))
    }
    #expect(files(rig) == [delivered.url.lastPathComponent])

    rig.counters.info = nil
    try await rig.camera.startRecording()
    await #expect(throws: MediaError.self) { _ = try await rig.camera.stopRecording() }
    #expect(rig.camera.state.fault == .recordFailed)
    #expect(files(rig) == [delivered.url.lastPathComponent])

    try await rig.camera.startRecording()
    rig.camera.cancelRecording()
    await eventually("ready again") { rig.camera.state.phase == .ready }
    #expect(files(rig) == [delivered.url.lastPathComponent])
  }

  @Test
  func inv_call_13_a_call_throws_a_running_video_recording_away_with_the_in_call_refusal_and_refuses_a_new_one() async throws {
    let rig = try makeRig(.moment)
    await ready(rig)
    try await rig.camera.startRecording()

    let call = try await rig.audio.link.call(.unmanaged)
    defer { call.end() }
    await eventually("recording revoked") { rig.camera.state.phase == .ready }
    do {
      _ = try await rig.camera.stopRecording()
      Issue.record("a recording survived the call")
    } catch let MediaError.Camera(reason, detail, _) {
      #expect((reason, detail) == (.inCall, cameraRefusalText(reason: .inCall)))
    }
    #expect(files(rig).isEmpty)
    await eventually("ready after the call") { rig.camera.state.phase == .ready }
    do {
      try await rig.camera.startRecording()
      Issue.record("a recording started during a call")
    } catch let MediaError.Camera(reason, _, _) {
      #expect(reason == .inCall)
    }
  }

  @Test(arguments: [AudioDriveSignal.startCall, .silenceMicrophone, .beginInterruption])
  func inv_audio_80_a_driven_system_event_throws_a_camera_take_away_with_its_refusal(_ signal: AudioDriveSignal) async throws {
    let rig = try makeRig(.moment)
    await ready(rig)
    try await rig.camera.startRecording()

    rig.audio.link.drive(signal)

    await eventually("recording revoked") { rig.camera.state.phase == .ready }
    let expected: CameraRefusal = switch signal {
    case .startCall: .inCall
    case .silenceMicrophone: .microphoneSilent
    default: .interrupted
    }
    do {
      _ = try await rig.camera.stopRecording()
      Issue.record("a recording survived \(signal)")
    } catch let MediaError.Camera(reason, _, _) {
      #expect(reason == expected)
    }
    #expect(files(rig).isEmpty)
  }

  @Test
  func inv_audio_43_a_take_without_microphone_sound_is_refused() async throws {
    let rig = try makeRig(.moment)
    await ready(rig)
    rig.backend.emit(.microphoneFlow(false))
    rig.backend.soundOnTake = false
    let silent = Task { try await rig.camera.startRecording() }
    await eventually("a silent microphone does not refuse the start: the take waits") { rig.backend.count("record back") == 1 }
    rig.backend.emit(.microphoneFlow(false))
    do {
      try await silent.value
      Issue.record("a silent take started")
    } catch let MediaError.Camera(reason, detail, _) {
      #expect((reason, detail) == (.microphoneSilent, cameraRefusalText(reason: .microphoneSilent)))
    }
    #expect(files(rig).isEmpty)
    rig.backend.soundOnTake = true
    await eventually("ready after the silent take") { rig.camera.state.phase == .ready }
  }

  @Test
  func inv_capture_84_a_take_that_lost_its_microphone_ends_and_delivers_the_sound_so_far() async throws {
    let rig = try makeRig(.moment)
    await ready(rig)
    rig.backend.emit(.microphoneFlow(true))
    try await rig.camera.startRecording()
    rig.backend.emit(.microphoneFlow(false))

    await eventually("the take ended") { rig.camera.state.phase != .recording }
    let clip = try await rig.camera.stopRecording()
    #expect(clip.durationMs > 0)
    #expect(rig.camera.state.fault == nil)
  }

  @Test
  func inv_capture_50_a_take_whose_ledger_is_covered_is_delivered_and_a_hole_file_is_never_read_for_the_sound() async throws {
    let judged = try await CoverageClips.writeJudged(audio: CoverageClips.hole)
    defer { try? FileManager.default.removeItem(at: judged.url) }
    let rig = try makeRig(.moment) { backend, _ in
      backend.clipFile = judged.url
      backend.coverage = takeCoverageOf(.covered)
    }
    await ready(rig)
    try await rig.camera.startRecording()
    let clip = try await rig.camera.stopRecording()
    #expect(clip.durationMs > 0, "the ledger is the judge: the file is not scanned after stop")
    #expect(rig.counters.rescanned.isEmpty, "no file call but the duration of the clip")
  }

  @Test
  func inv_capture_50_a_take_with_sound_and_no_ledger_is_refused_as_audio_track_broken() async throws {
    let rig = try makeRig(.moment) { backend, _ in backend.coverage = nil }
    await ready(rig)
    try await rig.camera.startRecording()
    do {
      _ = try await rig.camera.stopRecording()
      Issue.record("a take without a verdict was delivered")
    } catch let MediaError.Camera(reason, _, _) {
      #expect(reason == .audioTrackBroken)
    }
    await eventually("ready again") { rig.camera.state.phase == .ready }
    #expect(files(rig).isEmpty)
  }

  @Test
  func inv_capture_50_a_ledger_hole_is_refused_and_the_file_is_deleted() async throws {
    let judged = try await CoverageClips.writeJudged(audio: CoverageClips.hole)
    defer { try? FileManager.default.removeItem(at: judged.url) }
    let rig = try makeRig(.moment) { backend, _ in
      backend.clipFile = judged.url
      backend.coverage = judged.coverage
    }
    await ready(rig)
    try await rig.camera.startRecording()
    do {
      _ = try await rig.camera.stopRecording()
      Issue.record("a take with a hole in its sound was delivered")
    } catch let MediaError.Camera(reason, _, _) {
      #expect(reason == .audioTrackBroken)
    }
    await eventually("ready again") { rig.camera.state.phase == .ready }
    #expect(rig.camera.state.outcome == .refused(reason: .audioTrackBroken))
    #expect(files(rig).isEmpty)
    #expect(rig.backend.count("unbind") == 0)
  }

  @Test
  func inv_capture_50_a_real_clip_with_covered_sound_is_delivered_and_two_takes_on_one_source_each_start_on_their_own_pcm() async throws {
    let judged = try await CoverageClips.writeJudged(audio: CoverageClips.whole)
    let clip = judged.url
    defer { try? FileManager.default.removeItem(at: clip) }
    let rig = try makeRig(.moment) { backend, _ in
      backend.clipFile = clip
      backend.coverage = judged.coverage
    }
    await ready(rig)
    for _ in 0..<2 {
      try await rig.camera.startRecording()
      let delivered = try await rig.camera.stopRecording()
      #expect(FileManager.default.fileExists(atPath: delivered.url.path))
      try? FileManager.default.removeItem(at: delivered.url)
    }
    rig.backend.soundOnTake = false
    let third = Task { try await rig.camera.startRecording() }
    await eventually("the third take began") { rig.backend.count("record back") == 3 }
    #expect(rig.camera.state.phase != .recording)
    rig.backend.emit(.microphoneFlow(true))
    try await third.value
    #expect(rig.camera.state.phase == .recording)
  }

  @Test
  func inv_capture_33_after_a_silent_take_the_next_take_starts_only_on_its_own_fresh_pcm() async throws {
    let rig = try makeRig(.moment)
    await ready(rig)
    try await rig.camera.startRecording()
    rig.backend.emit(.microphoneFlow(false))
    await eventually("the take ended") { rig.camera.state.phase != .recording }
    _ = try? await rig.camera.stopRecording()

    rig.backend.soundOnTake = false
    let silent = Task { try await rig.camera.startRecording() }
    await eventually("the silent take began") { rig.backend.count("record back") == 2 }
    #expect(rig.camera.state.phase != .recording)
    rig.backend.emit(.microphoneFlow(true))
    try await silent.value
    await eventually("recording") { rig.camera.state.phase == .recording }
  }

  @Test
  func inv_capture_14_the_orphans_of_an_earlier_run_are_removed_and_other_files_stay() throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent("sweep-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    for name in ["capture-a.jpg", "capture-b.mp4", "keep.txt", "voice-a.m4a"] { try Data([1]).write(to: directory.appendingPathComponent(name)) }
    CaptureFiles.sweepOrphans(in: directory)
    #expect(((try? FileManager.default.contentsOfDirectory(atPath: directory.path)) ?? []).sorted() == ["keep.txt", "voice-a.m4a"])
  }

  @Test
  func inv_capture_16_commands_from_the_main_thread_run_no_hardware_core_or_file_work_on_it() async throws {
    let rig = try makeRig(.moment)
    await MainActor.run { rig.camera.attachPreview() }
    await eventually("ready") { rig.camera.state.phase == .ready }
    await MainActor.run {
      rig.camera.setFlash(true)
      rig.camera.zoom(by: 2)
      rig.camera.setLens(.front)
    }
    await eventually("front ready") { rig.camera.state.lens == .front && rig.camera.state.phase == .ready }
    try await rig.camera.startRecording()
    _ = try await rig.camera.stopRecording()
    await MainActor.run { rig.camera.detachPreview() }
    await eventually("unbound") { rig.backend.count("unbind") == 1 }
    #expect(rig.backend.mainThreadCalls == 0)
    #expect(rig.counters.mainThread == 0)
  }

  @Test
  func inv_capture_42_a_lens_switch_with_the_flash_on_reaches_ready_and_lights_the_torch_on_the_new_lens() async throws {
    let rig = try makeRig(.moment)
    await ready(rig)
    rig.camera.setFlash(true)
    await eventually("flash on") { rig.camera.state.flash }

    rig.camera.setLens(.front)

    await eventually("front ready") { rig.camera.state.lens == .front && rig.camera.state.phase == .ready }
    #expect(rig.camera.state.flash == true && rig.backend.count("torch true") == 2)
  }

  @Test
  func inv_capture_17_the_preview_leaving_the_window_and_the_background_release_the_camera_and_the_return_binds_again() async throws {
    let rig = try makeRig()
    await ready(rig)
    rig.camera.attachPreview()
    rig.camera.detachPreview()
    try await Task.sleep(for: .milliseconds(50))
    #expect((rig.backend.count("bind back"), rig.backend.count("unbind")) == (1, 0), "one of two previews leaving keeps the camera")
    rig.center.post(name: rig.signals.background, object: nil)
    await eventually("released in the background") { rig.backend.count("unbind") == 1 && rig.camera.state.phase == .idle }
    rig.center.post(name: rig.signals.foreground, object: nil)
    await eventually("bound again") { rig.backend.count("bind back") == 2 && rig.camera.state.phase == .ready }
    rig.camera.detachPreview()
    await eventually("unbound") { rig.backend.count("unbind") == 2 }
    rig.center.post(name: rig.signals.foreground, object: nil)
    try await Task.sleep(for: .milliseconds(50))
    #expect(rig.backend.count("bind back") == 2, "the foreground without a preview binds nothing")
  }

  @Test
  func inv_capture_18_an_idle_camera_holds_no_session_no_ticker_and_no_frame_output() async throws {
    let rig = try makeRig(.moment)
    #expect(await rig.camera.tickerTask().map { _ in true } == nil)
    #expect(rig.backend.log.isEmpty && rig.backend.tap == nil && rig.counters.sessionsTaken == 0)
    await ready(rig)
    try await rig.camera.startRecording()
    var readsEvent = rig.backend.readEvents.stream.makeAsyncIterator()
    _ = await readsEvent.next()
    let ticker = try #require(await rig.camera.tickerTask())
    _ = try await rig.camera.stopRecording()
    await ticker.value
    let reads = rig.backend.recordedReads
    #expect(await rig.camera.tickerTask().map { _ in true } == nil)
    #expect(rig.backend.recordedReads == reads, "the ticker stops with the recording")
  }

  @Test
  func inv_capture_19_a_reset_restarts_once_and_the_second_one_fails_until_the_preview_returns() async throws {
    let rig = try makeRig()
    await ready(rig)
    rig.backend.emit(.fault(kind: .reset))
    await eventually("rebound") { rig.backend.count("bind back") == 2 && rig.camera.state.phase == .ready }
    #expect(rig.backend.count("unbind") == 1)
    rig.backend.emit(.fault(kind: .reset))
    await eventually("failed") { rig.camera.state.phase == .failed }
    #expect(rig.camera.state.fault == .busy)
    rig.camera.detachPreview()
    rig.camera.attachPreview()
    await eventually("ready again") { rig.camera.state.phase == .ready }
    rig.backend.emit(.fault(kind: .reset))
    await eventually("restarted again") { rig.backend.count("bind back") >= 4 && rig.camera.state.phase == .ready }
  }

  @Test
  func inv_capture_20_a_recording_starts_on_the_event_of_the_hardware_ends_by_the_limit_and_keeps_its_clip() async throws {
    let rig = try makeRig(.moment)
    await ready(rig)
    try await rig.camera.startRecording()
    #expect(rig.camera.state.phase == .recording)
    await eventually("length published") { rig.camera.state.recordedMs == 700 }
    rig.backend.emit(.recordFinished(end: .limitReached))
    await eventually("settled") { rig.camera.state.phase == .ready }
    let clip = try await rig.camera.stopRecording()
    #expect((clip.durationMs, clip.url.pathExtension) == (2000, "mp4"))
    #expect(FileManager.default.fileExists(atPath: clip.url.path))
    await #expect(throws: MediaError.self) { _ = try await rig.camera.startRecording(); _ = try await rig.camera.startRecording() }
  }

  @Test
  func inv_capture_20_a_lens_change_during_a_recording_throws_it_away_and_binds_the_other_side() async throws {
    let rig = try makeRig(.moment)
    await ready(rig)
    try await rig.camera.startRecording()
    let recorded = try #require(rig.backend.recordingURL)
    rig.camera.setLens(.front)
    await eventually("front bound") { rig.backend.count("bind front") == 1 && rig.camera.state.phase == .ready }
    #expect(rig.camera.state.lens == .front)
    #expect(!FileManager.default.fileExists(atPath: recorded.path))
    await #expect(throws: MediaError.self) { _ = try await rig.camera.stopRecording() }
  }

  @Test
  func inv_capture_20_the_preview_leaving_during_a_recording_throws_the_clip_away_and_frees_the_camera_and_the_microphone() async throws {
    let rig = try makeRig(.moment)
    await ready(rig)
    try await rig.camera.startRecording()
    let recorded = try #require(rig.backend.recordingURL)
    let stopping = Task { try await rig.camera.stopRecording() }
    rig.camera.detachPreview()
    await eventually("unbound") { rig.backend.count("unbind") == 1 && rig.counters.released == 1 }
    #expect(!FileManager.default.fileExists(atPath: recorded.path))
    await #expect(throws: MediaError.self) { _ = try await stopping.value }
    #expect(rig.camera.state.phase == .idle && rig.camera.state.recordedMs == 0)
    await eventually("microphone given back") { !rig.audio.isRecording }
  }

  @Test
  func inv_capture_21_a_profile_with_sound_takes_the_recording_focus_on_open_and_gives_it_back_on_release() async throws {
    let rig = try makeRig(.moment)
    #expect(!rig.audio.isRecording)
    await ready(rig)
    #expect(rig.audio.isRecording)
    rig.camera.detachPreview()
    await eventually("given back") { !rig.audio.isRecording }
    let still = try makeRig(.photo)
    await ready(still)
    #expect(!still.audio.isRecording, "a photo profile does not take the microphone")
  }

  @Test
  func inv_capture_22_the_frame_output_stands_only_while_somebody_listens_and_the_rate_is_kept() async throws {
    let rig = try makeRig()
    await ready(rig)
    #expect(rig.backend.tap == nil)
    let received = Received()
    let observation = rig.camera.observeFrames(maxPerSecond: 10) { _ in received.add() }
    await eventually("tap installed") { rig.backend.tap != nil }
    var pixels: CVPixelBuffer?
    CVPixelBufferCreate(nil, 2, 2, kCVPixelFormatType_32BGRA, nil, &pixels)
    let buffer = try #require(pixels)
    for millis: UInt64 in [1000, 1040, 1099, 1100, 1150, 1250] { rig.backend.tap?(buffer, millis) }
    #expect(received.count == 3, "the frames at 1000, 1100 and 1250 pass a gate of 10 a second")
    observation.cancel()
    await eventually("tap removed") { rig.backend.tap == nil }
  }

  @Test
  func inv_capture_41_a_frame_observer_that_arrives_during_a_take_gets_frames_after_it_without_touching_the_recording() async throws {
    let rig = try makeRig(.moment)
    await ready(rig)
    try await rig.camera.startRecording()
    await eventually("recording") { rig.camera.state.phase == .recording }
    let binds = rig.backend.log.filter { $0.hasPrefix("bind") }.count

    let observation = rig.camera.observeFrames(maxPerSecond: 10) { _ in }

    #expect(rig.backend.tap == nil, "the frame output was added to the running recording")
    #expect(rig.backend.log.filter { $0.hasPrefix("bind") }.count == binds, "the running recording was bound again")
    _ = try await rig.camera.stopRecording()
    await eventually("the frame output stands after the take") { rig.backend.tap != nil }
    observation.cancel()
  }

  @Test @MainActor
  func the_preview_layer_is_a_visible_clear_sublayer() throws {
    let rig = try makeRig()
    let view = CameraPreviewLayerView(camera: rig.camera)
    #expect(!(view.layer is CameraPreviewLayer))
    #expect(view.previewLayer.superlayer === view.layer)
    #expect(!view.previewLayer.isHidden)
    #expect(view.previewLayer.backgroundColor == UIColor.clear.cgColor)
    view.frame = CGRect(x: 0, y: 0, width: 120, height: 80)
    view.layoutIfNeeded()
    #expect(view.previewLayer.frame == view.bounds)
    view.applyPreviewing(true)
    view.applyPreviewing(false)
    #expect(!view.previewLayer.isHidden)
  }

  @Test @MainActor
  func inv_capture_23_the_preview_fills_the_frame_and_a_pinch_zooms_by_the_step_and_the_view_attaches_by_the_window() async throws {
    let rig = try makeRig()
    let view = CameraPreviewLayerView(camera: rig.camera)
    #expect(view.previewLayer.videoGravity == .resizeAspectFill)
    #expect(view.gestureRecognizers?.contains { $0 is UIPinchGestureRecognizer } == true)
    view.zoomsByPinch = false
    #expect(view.gestureRecognizers?.contains { $0 is UIPinchGestureRecognizer } != true)
    view.sync(onScreen: true)
    await eventually("ready") { rig.camera.state.phase == .ready }
    view.pinch(scale: 1, state: .began)
    view.pinch(scale: 1.5, state: .changed)
    view.pinch(scale: 3, state: .changed)
    await eventually("zoomed") { rig.backend.count("zoom 3.0") == 1 }
    #expect(rig.backend.count("zoom 1.5") == 1)
    view.sync(onScreen: false)
    await eventually("unbound") { rig.backend.count("unbind") == 1 }
  }

  @Test @MainActor
  func a_mounted_preview_that_gets_another_camera_moves_the_demand_to_it() async throws {
    let first = try makeRig()
    let second = try makeRig()
    let layers = LayerLog()
    let model = CameraSwap(camera: first.camera)
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
    let host = UIHostingController(rootView: SwapCell(model: model, layers: layers))
    window.rootViewController = host
    window.makeKeyAndVisible()
    await eventually("the first camera is ready") { first.camera.state.phase == .ready }
    #expect(layers.count == 1)
    model.camera = second.camera
    await eventually("the second camera got the preview") { second.camera.state.phase == .ready }
    await eventually("the first camera was released") { first.backend.count("unbind") == 1 }
    #expect(second.backend.count("attachPreview") == 1)
    #expect(first.backend.count("detachPreview") == 1)
    #expect(layers.count == 2)
    window.isHidden = true
    window.rootViewController = nil
  }

  @Test @MainActor
  func with_media_camera_state_follows_the_camera_it_is_given_now() async throws {
    let first = try makeRig()
    let second = try makeRig()
    await ready(second)
    let model = CameraSwap(camera: first.camera)
    let seen = PhaseLog()
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
    window.rootViewController = UIHostingController(rootView: StateCell(model: model, seen: seen))
    window.makeKeyAndVisible()
    await eventually("the first camera is shown") { seen.last == first.camera.state.phase }
    #expect(first.camera.state.phase != .ready)
    model.camera = second.camera
    await eventually("the second camera is shown") { seen.last == .ready }
    await MainActor.run { first.camera.attachPreview() }
    await eventually("the first camera is ready") { first.camera.state.phase == .ready }
    window.isHidden = true
    window.rootViewController = nil
  }

  @Test @MainActor
  func inv_capture_29_the_preview_joins_the_session_only_on_the_backend_queue() async throws {
    let rig = try makeRig()
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 390, height: 844))
    for round in 0..<5 {
      let view = CameraPreviewLayerView(camera: rig.camera)
      window.addSubview(view)
      view.frame = window.bounds
      view.layoutIfNeeded()
      view.frame = CGRect(x: 0, y: 0, width: 844, height: 390)
      view.layoutIfNeeded()
      #expect(view.previewLayer.sessionJoinsOnMain == 0, "round \(round): the main thread joined the layer to the session")
      view.removeFromSuperview()
    }
    await eventually("every layer was registered and let go") { rig.backend.count("attachPreview") == 5 && rig.backend.count("detachPreview") == 5 }
    #expect(rig.backend.mainThreadCalls == 0)
  }

  @Test
  func inv_capture_24_a_camera_that_the_app_lets_go_releases_the_session() async throws {
    let backend: FakeBackend
    let counters: Counters
    do {
      let rig = try makeRig()
      await ready(rig)
      backend = rig.backend
      counters = rig.counters
    }
    await eventually("unbound by deinit") { backend.count("unbind") == 1 && counters.released == 1 }
  }

  @Test
  func inv_capture_25_a_recording_hands_the_stabilization_of_its_profile_to_the_backend() async throws {
    for (profile, expected) in [(CaptureProfile.circle, VideoStabilization.cinematic), (.moment, .off)] {
      let rig = try makeRig(profile)
      await ready(rig)
      try await rig.camera.startRecording()
      #expect(rig.backend.recordedProfile?.stabilization == expected)
    }
  }

  @Test
  func inv_capture_27_a_circle_profile_with_a_least_take_of_500_ms_is_built_and_reaches_the_backend() async throws {
    var profile = CaptureProfile.circle
    profile.minTakeMs = 500
    let rig = try makeRig(profile)
    await ready(rig)
    try await rig.camera.startRecording()
    #expect(rig.backend.recordedProfile?.minTakeMs == 500)
    #expect(rig.backend.recordedProfile?.stabilization == .cinematic)
  }

  @Test
  func inv_capture_28_a_too_short_recording_carries_its_length_in_the_error() async throws {
    let rig = try makeRig(.moment)
    await ready(rig)
    rig.counters.info = MediaInfo(durationMs: 400, frame: nil)
    try await rig.camera.startRecording()
    do {
      _ = try await rig.camera.stopRecording()
      Issue.record("a recording of 400 ms was delivered")
    } catch let MediaError.Camera(reason, _, recordedMs) {
      #expect((reason, recordedMs) == (.tooShort, 400))
    }

    rig.counters.info = nil
    await eventually("ready again") { rig.camera.state.phase == .ready }
    try await rig.camera.startRecording()
    do {
      _ = try await rig.camera.stopRecording()
      Issue.record("a recording without a length was delivered")
    } catch let MediaError.Camera(reason, _, recordedMs) {
      #expect((reason, recordedMs) == (.recordFailed, nil))
    }
  }

  @Test
  func inv_capture_09_a_bad_profile_makes_no_camera() {
    let broken = CaptureProfile(mode: .video, videoSize: nil, photoSize: nil, fps: 30, maxDurationMs: 0, minTakeMs: 0, audio: true, mirrorFrontVideo: false, stabilization: .off, photoQuality: 90, reducesRedEye: false)
    #expect(throws: MediaError.self) { _ = try MediaCamera(profile: broken) }
  }

  @Test
  func inv_capture_31_the_clip_of_a_take_is_given_once() async throws {
    let rig = try makeRig(.moment)
    await ready(rig)
    try await rig.camera.startRecording()
    rig.backend.emit(.recordFinished(end: .limitReached))
    await eventually("settled") { rig.camera.state.phase == .ready }
    let clip = try await rig.camera.stopRecording()
    #expect(FileManager.default.fileExists(atPath: clip.url.path))
    await #expect(throws: MediaError.self) { _ = try await rig.camera.stopRecording() }
  }

  // C1
  @Test
  func inv_capture_77_a_take_ended_by_the_limit_shows_its_captured_outcome_without_stop() async throws {
    let rig = try makeRig(.moment)
    await ready(rig)
    let states = Collected(rig.camera.states)
    try await rig.camera.startRecording()
    let take = try #require(rig.camera.state.take)

    rig.backend.emit(.recordFinished(end: .limitReached))

    await eventually("the outcome reached the app") { states.last?.outcome != nil }
    #expect(states.last?.outcome == .captured(take: take))
    #expect((states.last?.take, states.last?.phase) == (take, .ready))
    #expect(files(rig).count == 1 && files(rig).first?.hasSuffix(".mp4") == true, "the clip stays without a stop: \(files(rig))")
  }

  // C2
  @Test(arguments: [("interruption", CameraRefusal.interrupted), ("too short", .tooShort), ("failure", .recordFailed)])
  func inv_capture_36_a_thrown_away_take_shows_its_refusal_and_fault_without_stop(cause: String, reason: CameraRefusal) async throws {
    let rig = try makeRig(.moment)
    await ready(rig)
    let states = Collected(rig.camera.states)
    rig.backend.emit(.microphoneFlow(true))
    try await rig.camera.startRecording()
    let take = try #require(rig.camera.state.take)

    switch cause {
    case "interruption": rig.audio.port.send(.interruptionBegan)
    case "too short":
      rig.counters.info = MediaInfo(durationMs: 300, frame: nil)
      rig.backend.emit(.recordFinished(end: .limitReached))
    default:
      rig.counters.info = nil
      rig.backend.emit(.recordFinished(end: .limitReached))
    }

    await eventually("\(cause): the outcome reached the app") { states.last?.outcome != nil }
    #expect(states.last?.outcome == .refused(reason: reason), "\(cause)")
    #expect((states.last?.take, states.last?.fault) == (take, reason), "\(cause)")
    #expect(files(rig).isEmpty, "\(cause)")
  }

  // C3
  @Test
  func inv_capture_37_a_reset_during_a_take_shows_the_lost_take_and_the_camera_opens_again() async throws {
    let rig = try makeRig(.moment)
    await ready(rig)
    let states = Collected(rig.camera.states)
    try await rig.camera.startRecording()
    let take = try #require(rig.camera.state.take)

    rig.backend.emit(.fault(kind: .reset))

    await eventually("recovered") { states.values.contains { $0.outcome == .lost(reason: .interrupted) } && states.last?.phase == .ready }
    #expect((states.last?.take, states.last?.outcome) == (take, .lost(reason: .interrupted)))
    #expect(states.values.filter { $0.outcome == .lost(reason: .interrupted) }.map(\.take).allSatisfy { $0 == take })
    #expect(files(rig).isEmpty)
  }

  // C5 is in inv_ios_04. C4 is an Android row.


  // CAPTURE-65: the hold of the session goes back after the hardware stopped.
  @Test
  func inv_capture_65_the_memory_of_the_session_goes_back_only_after_the_hardware_stopped() async throws {
    let rig = try makeRig { backend, _ in backend.deferUnbind = true }
    await ready(rig)
    rig.camera.detachPreview()
    await eventually("the hardware stop started") { rig.backend.count("unbind") == 1 }
    #expect(rig.counters.released == 0, "the stop of the hardware is not over")
    rig.backend.finishUnbind()
    await eventually("the memory is back") { rig.counters.released == 1 }
  }

  // CAPTURE-66: a cancelled start throws away its own take.
  @Test
  func inv_capture_66_a_cancelled_start_recording_cancels_its_take_and_leaves_the_camera_ready() async throws {
    let rig = try makeRig(.moment) { backend, _ in backend.holdRecordStart = true }
    await ready(rig)
    let start = Task { try await rig.camera.startRecording() }
    await eventually("the hardware was asked") { rig.backend.count("record back") == 1 }
    start.cancel()
    await #expect(throws: CancellationError.self) { try await start.value }
    await eventually("the take is gone") { rig.backend.count("endRecording") == 1 && rig.camera.state.phase == .ready }
    #expect(files(rig).isEmpty)
    rig.backend.holdRecordStart = false
    try await rig.camera.startRecording()
    #expect(rig.camera.state.phase == .recording)
  }

  // CAPTURE-67: a torch or zoom the system rejects drops only its own field.
  @Test
  func inv_capture_67_a_torch_the_system_rejects_after_open_keeps_the_open_step_and_drops_only_the_flash() async throws {
    let rig = try makeRig(.moment) { backend, _ in
      backend.commandFailure = NSError(domain: AVFoundationErrorDomain, code: -11852, userInfo: [NSLocalizedDescriptionKey: "lock failed"])
    }
    rig.camera.setFlash(true)
    await ready(rig)
    #expect(rig.camera.state.flash == false)
    try await rig.camera.startRecording()
    #expect(rig.camera.state.phase == .recording)
  }

  // CAPTURE-69: a refused microphone opens no audio demand.
  @Test
  func inv_capture_69_a_refused_microphone_opens_no_audio_input_and_changes_no_audio_session() async throws {
    let rig = try makeRig(.moment, permission: DeniedMicrophone()) { backend, _ in backend.nativeMicrophone = true }
    rig.camera.attachPreview()
    await eventually("the camera is refused") { rig.camera.state.fault == .permissionDenied }
    #expect(rig.backend.count("mic start") == 0)
    #expect(!rig.audio.port.log.contains { $0.hasPrefix("apply") })
  }

  // CAPTURE-70: the clip of a take stays until its owner takes it, and it keeps the lens of its take.
  @Test
  func inv_capture_70_a_delivered_clip_survives_the_next_take_and_keeps_the_lens_of_its_take() async throws {
    let rig = try makeRig(.moment)
    await ready(rig)
    try await rig.camera.startRecording()
    rig.backend.emit(.recordFinished(end: .limitReached))
    await eventually("the take is captured") { rig.camera.state.outcome != nil }
    rig.camera.setLens(.front)
    let take = try #require(rig.camera.state.take)
    await eventually("ready on the other lens") { rig.camera.state.phase == .ready && rig.camera.state.lens == .front }
    try await rig.camera.startRecording()
    let clip = try #require(rig.camera.clip(take: take))
    #expect(FileManager.default.fileExists(atPath: clip.url.path))
    #expect(clip.lens == .back)
    #expect(clip.end == .limitReached)
    #expect(rig.camera.clip(take: take) == nil)
  }

  // CAPTURE-80: a take whose microphone stopped delivering is delivered with the end it had.
  @Test
  func inv_capture_80_a_clip_of_a_take_that_lost_the_microphone_carries_that_end() async throws {
    let rig = try makeRig(.moment)
    await ready(rig)
    try await rig.camera.startRecording()
    let take = try #require(rig.camera.state.take)
    rig.backend.emit(.recordFinished(end: .microphoneLost))
    await eventually("the take is captured") { rig.camera.state.outcome != nil }
    let clip = try #require(rig.camera.clip(take: take))
    #expect(clip.end == .microphoneLost)
  }

  // C6
  @Test(arguments: ["flash", "zoom"])
  func inv_capture_40_a_command_the_system_rejects_is_a_failure_event_and_the_state_stays_true(command: String) async throws {
    let rig = try makeRig(.moment)
    await ready(rig)
    let failures = Collected(Media.failures)
    rig.backend.commandFailure = NSError(domain: AVFoundationErrorDomain, code: -11852, userInfo: [NSLocalizedDescriptionKey: "lock failed"])

    if command == "flash" { rig.camera.setFlash(true) } else { rig.camera.zoom(by: 2) }

    await eventually("\(command): the failure reached the app") { failures.values.contains { $0.source == .camera } }
    let failure = try #require(failures.values.first { $0.source == .camera })
    #expect((failure.domain, failure.code) == (AVFoundationErrorDomain, -11852))
    #expect((rig.camera.state.flash, rig.camera.state.zoom) == (false, 1), "\(command)")
  }
}

extension GlobalMedia {
  @Suite(.serialized, .timeLimit(.minutes(2)))
  struct LayerCloseTests {
    // D08, D13 and the close rows of C5, V2, K4, M4
    @Test
    func inv_ios_04_closing_the_layer_ends_the_camera_the_recorder_the_call_the_volume_and_the_availability() async throws {
      let app = try await AppLayer.open()
      let backend = FakeBackend()
      var services = CameraServices.live
      services.directory = FileManager.default.temporaryDirectory.appendingPathComponent("camera-\(UUID().uuidString)")
      services.center = NotificationCenter()
      services.signals = names
      let camera = try MediaCamera(profile: .photo, lens: .back, backend: backend, services: services)
      let cameraStates = Collected(camera.states)
      camera.attachPreview()
      await eventually("preview ready") { cameraStates.last?.phase == .ready }
      let recorder = MediaVoiceRecorder(capture: WritingCapture(), maxDurationMs: 60_000)
      let voice = Collected(recorder.states)
      let target = temporaryFile("m4a")
      try await recorder.start(to: target)
      let raises = Collected(try #require(await Media.audio?.volumeRaises()))
      let availability = Collected(app.media.microphoneAvailability)
      await eventually("observing") { app.session.volumeObserved && !availability.values.isEmpty }

      await app.close()

      await eventually("camera closed") { cameraStates.last?.phase == .closed }
      #expect(backend.count("unbind") == 1)
      let later = Collected(camera.states)
      await eventually("a new subscription of a closed camera ends") { later.finished }
      #expect(later.last?.phase == .closed)
      await eventually("the take ended") { voice.last?.outcome != nil }
      #expect(voice.last == VoiceState(take: 1, phase: .idle, outcome: .refused(reason: .interrupted), recordedMs: 0))
      #expect(!FileManager.default.fileExists(atPath: target.path))
      await eventually("volume and availability streams ended") { raises.finished && availability.finished && !app.session.volumeObserved }
      let late = Collected(app.media.microphoneAvailability)
      await eventually("a read after the close ends at once") { late.finished }
      #expect(late.values.isEmpty)

      let calling = try await AppLayer.open()
      let call = try await calling.media.callAudio(.unmanaged)
      let callStates = Collected(call.states)
      let route = Collected(call.route)

      await calling.close()

      await eventually("call ended") { callStates.finished && route.finished }
      #expect(callStates.last == .ended)
      await #expect(throws: MediaError.self) { _ = try await calling.media.callAudio(.unmanaged) }
    }
  }
}

private final class Received: @unchecked Sendable {
  private let lock = NSLock()
  private var value = 0
  var count: Int { lock.withLock { value } }
  func add() { lock.withLock { value += 1 }; TestSignal.shared.fire() }
}
#endif
