#if os(iOS)
import AVFoundation
import Foundation
import MediaLayer
import Testing
import UIKit
@testable import MediaLayerUI

private final class FakeBackend: CaptureBackend, @unchecked Sendable {
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
  var photoSize: PixelSize? { PixelSize(width: 3000, height: 4000) }

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
    case let .refuses(reason): emit(.refuse(reason: reason))
    case .silent: break
    }
  }

  func unbind() { note("unbind") }
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
    try? Data([1, 2, 3]).write(to: url)
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

  func setTorch(_ on: Bool) { note("torch \(on)") }
  func setZoom(_ ratio: Float) { note("zoom \(ratio)") }
  func setFrameTap(_ tap: (@Sendable (CVPixelBuffer, UInt64) -> Void)?) { lock.withLock { self.tap = tap }; TestSignal.shared.fire() }
}

private final class Counters: @unchecked Sendable {
  private let lock = NSLock()
  private var taken = 0
  private var given = 0
  private var uprightQualities: [Double] = []
  private var mainCalls = 0
  var denySessions = false
  var denyShots = false
  var info: MediaInfo? = MediaInfo(durationMs: 2000, frame: ImageSize(width: 720, height: 1280))

  var sessionsTaken: Int { lock.withLock { taken } }
  var released: Int { lock.withLock { given } }
  var qualities: [Double] { lock.withLock { uprightQualities } }
  var mainThread: Int { lock.withLock { mainCalls } }

  func took() { lock.withLock { taken += 1 }; TestSignal.shared.fire() }
  func gave() { lock.withLock { given += 1 }; TestSignal.shared.fire() }
  func upright(_ quality: Double) { lock.withLock { uprightQualities.append(quality) }; TestSignal.shared.fire() }
  func checkThread() { if Thread.isMainThread { lock.withLock { mainCalls += 1 } } }

  private var parked: CheckedContinuation<Void, Error>?
  private var straightenedURLs: [URL] = []
  private var cancelledStraighten = false
  var holdVideo = false
  var straightened: [URL] { lock.withLock { straightenedURLs } }
  var straightenCancelled: Bool { lock.withLock { cancelledStraighten } }

  func straighten(_ url: URL) async throws {
    let hold = lock.withLock { straightenedURLs.append(url); return holdVideo }
    TestSignal.shared.fire()
    guard hold else { return }
    try await withTaskCancellationHandler {
      try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
        let cancelled = lock.withLock { () -> Bool in
          if cancelledStraighten { return true }
          parked = continuation
          return false
        }
        if cancelled { continuation.resume(throwing: CancellationError()) }
      }
    } onCancel: {
      let continuation = lock.withLock { () -> CheckedContinuation<Void, Error>? in
        cancelledStraighten = true
        defer { parked = nil }
        return parked
      }
      continuation?.resume(throwing: CancellationError())
    }
  }
}

private final class StateObservation {
  let task: Task<Void, Never>
  init(_ states: AsyncStream<CameraState>) {
    task = Task { for await _ in states { TestSignal.shared.fire() } }
  }
  deinit { task.cancel() }
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
  foreground: Notification.Name("camera.test.foreground"),
  outputLost: Notification.Name("camera.test.output")
)

private func jpeg() -> Data {
  let format = UIGraphicsImageRendererFormat()
  format.scale = 1
  return UIGraphicsImageRenderer(size: CGSize(width: 3, height: 5), format: format).jpegData(withCompressionQuality: 0.9) { context in
    UIColor.red.setFill()
    context.fill(CGRect(x: 0, y: 0, width: 3, height: 5))
  }
}

private func makeRig(_ profile: CaptureProfile = .photo, lens: CameraLens = .back, configure: (FakeBackend, Counters) -> Void = { _, _ in }) throws -> Rig {
  let backend = FakeBackend()
  let counters = Counters()
  configure(backend, counters)
  let center = NotificationCenter()
  let directory = FileManager.default.temporaryDirectory.appendingPathComponent("camera-\(UUID().uuidString)")
  let audio = try TestAudioRig()
  let image = jpeg()
  let services = CameraServices(
    admitSession: { _ in
      counters.checkThread()
      if counters.denySessions { throw MediaError.Overloaded(detail: "no memory") }
      counters.took()
      return { counters.gave() }
    },
    admitShot: { _ in
      counters.checkThread()
      if counters.denyShots { throw MediaError.Overloaded(detail: "no memory") }
      counters.took()
      return { counters.gave() }
    },
    link: { audio.link },
    clipInfo: { _ in
      counters.checkThread()
      return counters.info
    },
    uprightVideo: { url in try await counters.straighten(url) },
    upright: { raw, quality in
      counters.checkThread()
      counters.upright(quality)
      let made = raw.deletingLastPathComponent().appendingPathComponent("\(UUID().uuidString).jpg")
      try image.write(to: made)
      return made
    },
    directory: directory,
    center: center,
    signals: names,
    clock: { 0 }
  )
  let camera = try MediaCamera(profile: profile, lens: lens, backend: backend, services: services)
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
    let rig = try makeRig { _, counters in counters.denySessions = true }
    rig.camera.attachPreview()
    await eventually("failed") { rig.camera.state.phase == .failed }
    #expect(rig.camera.state.fault == .memory)
    #expect(rig.backend.log.filter { $0.hasPrefix("bind") }.isEmpty)
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
    rig.counters.denyShots = true
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

    Media.calls.hold()
    defer { Media.calls.release() }
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
  func inv_capture_29_a_finished_take_is_straightened_while_stopping_and_a_cancel_then_throws_it_away() async throws {
    let rig = try makeRig(.moment) { _, counters in counters.holdVideo = true }
    await ready(rig)
    try await rig.camera.startRecording()
    let recorded = try #require(rig.backend.recordingURL)
    rig.backend.emit(.recordFinished(end: .limitReached))
    await eventually("straightening while stopping") { rig.counters.straightened == [recorded] && rig.camera.state.phase == .stopping }
    rig.camera.cancelRecording()
    await eventually("ready after the cancel") { rig.camera.state.phase == .ready }
    #expect(rig.counters.straightenCancelled)
    #expect(!FileManager.default.fileExists(atPath: recorded.path))
    await #expect(throws: MediaError.self) { _ = try await rig.camera.stopRecording() }
  }

  @Test
  func inv_capture_31_the_clip_of_a_take_is_given_once() async throws {
    let rig = try makeRig(.moment)
    await ready(rig)
    try await rig.camera.startRecording()
    let recorded = try #require(rig.backend.recordingURL)
    rig.backend.emit(.recordFinished(end: .limitReached))
    await eventually("settled") { rig.camera.state.phase == .ready }
    let clip = try await rig.camera.stopRecording()
    #expect(FileManager.default.fileExists(atPath: clip.url.path))
    #expect(rig.counters.straightened == [recorded])
    await #expect(throws: MediaError.self) { _ = try await rig.camera.stopRecording() }
  }
}

private final class Received: @unchecked Sendable {
  private let lock = NSLock()
  private var value = 0
  var count: Int { lock.withLock { value } }
  func add() { lock.withLock { value += 1 }; TestSignal.shared.fire() }
}
#endif
