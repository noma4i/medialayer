import AVFoundation
import MediaLayer
@testable import MediaLayerProbe
@testable import MediaLayerUI
import UIKit
import XCTest

/// Every check runs on the phone with the real camera and microphone, through the public API the app uses. The only seam is `MediaTrace.observer`, which reads the library's own trace lines.
final class ProbeTests: XCTestCase {
  private static let lock = NSLock()
  /// One folder per run: scripts/check_device_ios.sh passes PROBE_RUN, so a relaunch of the app after a failed test keeps the clips, probe.log and trace.log of the earlier tests.
  private static let folder: URL = {
    let run = ProcessInfo.processInfo.environment["PROBE_RUN"] ?? "manual"
    let url = FileManager.default.urls(for: .documentDirectory, in: .userDomainMask)[0].appendingPathComponent("probe").appendingPathComponent(run)
    try? FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    return url
  }()

  override class func setUp() {
    UserDefaults.standard.set(true, forKey: "MLTrace")
    MediaTrace.observer = { line in
      lock.withLock {
        let file = folder.appendingPathComponent("trace.log")
        if let handle = try? FileHandle(forWritingTo: file) {
          _ = try? handle.seekToEnd(); handle.write(Data((line + "\n").utf8)); try? handle.close()
        } else {
          try? Data((line + "\n").utf8).write(to: file)
        }
      }
    }
  }

  override func setUp() { continueAfterFailure = false }

  private static var collected: [String] {
    lock.withLock { ((try? String(contentsOf: folder.appendingPathComponent("trace.log"), encoding: .utf8)) ?? "").split(separator: "\n").map(String.init) }
  }

  private static var takeCount: Int {
    ((try? FileManager.default.contentsOfDirectory(atPath: folder.path)) ?? []).filter { $0.hasPrefix("take_") && $0.hasSuffix(".mp4") }.count
  }

  private func probe(_ name: String, _ values: [(String, Any)]) {
    let text = "PROBE \(name) " + values.map { "\($0.0)=\($0.1)" }.joined(separator: " ")
    print(text)
    if let handle = try? FileHandle(forWritingTo: Self.folder.appendingPathComponent("probe.log")) {
      _ = try? handle.seekToEnd(); handle.write(Data((text + "\n").utf8)); try? handle.close()
    } else {
      try? Data((text + "\n").utf8).write(to: Self.folder.appendingPathComponent("probe.log"))
    }
    Task { @MainActor in ProbeScreen.shared.report += "\n" + text }
  }

  private func permissions() async {
    for type in [AVMediaType.video, .audio] {
      if AVCaptureDevice.authorizationStatus(for: type) == .notDetermined { _ = await AVCaptureDevice.requestAccess(for: type) }
      XCTAssertEqual(AVCaptureDevice.authorizationStatus(for: type), .authorized, "grant camera and microphone to MediaLayerProbe on the phone, then run again")
    }
  }

  /// Opens a camera on the probe screen and waits for READY. Returns the camera and the wait in ms.
  @MainActor private func open(_ profile: CaptureProfile, _ lens: CameraLens) async throws -> (MediaCamera, Int) {
    await permissions()
    _ = try await ProbeApp.opening.value
    let camera = try MediaCamera(profile: profile, lens: lens)
    let started = Date()
    ProbeScreen.shared.camera = camera
    while !camera.state.isReady {
      if camera.state.phase == .failed || Date().timeIntervalSince(started) > 30 { XCTFail("camera not ready: \(camera.state)"); break }
      try await Task.sleep(nanoseconds: 20_000_000)
    }
    return (camera, Int(Date().timeIntervalSince(started) * 1000))
  }

  @MainActor private func close(_ camera: MediaCamera) async throws {
    ProbeScreen.shared.camera = nil
    try await Task.sleep(nanoseconds: 700_000_000)
    _ = camera
  }

  private struct Facts { var size = CGSize.zero; var identity = false; var audioTracks = 0; var videoSeconds = 0.0; var audioSeconds = 0.0 }

  private func facts(of url: URL) async throws -> Facts {
    let asset = AVURLAsset(url: url)
    var facts = Facts()
    if let video = try await asset.loadTracks(withMediaType: .video).first {
      let transform = try await video.load(.preferredTransform)
      let shown = try await video.load(.naturalSize).applying(transform)
      facts.size = CGSize(width: abs(shown.width), height: abs(shown.height))
      facts.identity = transform == .identity
      facts.videoSeconds = try await video.load(.timeRange).duration.seconds
    }
    let audio = try await asset.loadTracks(withMediaType: .audio)
    facts.audioTracks = audio.count
    if let first = audio.first { facts.audioSeconds = try await first.load(.timeRange).duration.seconds }
    let generator = AVAssetImageGenerator(asset: asset)
    generator.appliesPreferredTrackTransform = true
    let image = try await generator.image(at: .zero).image
    try UIImage(cgImage: image).pngData()?.write(to: Self.folder.appendingPathComponent(url.deletingPathExtension().lastPathComponent + ".png"))
    return facts
  }

  @MainActor private func take(_ name: String, profile: CaptureProfile, lens: CameraLens, seconds: Double) async throws {
    let (camera, _) = try await open(profile, lens)
    try await camera.startRecording()
    try await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
    let stopped = Date()
    let clip = try await camera.stopRecording()
    let stopToClipMs = Int(Date().timeIntervalSince(stopped) * 1000)
    let kept = Self.folder.appendingPathComponent("\(name).mp4")
    try? FileManager.default.removeItem(at: kept)
    try FileManager.default.copyItem(at: clip.url, to: kept)
    let facts = try await facts(of: kept)
    probe(name, [("stopToClipMs", stopToClipMs), ("size", "\(Int(facts.size.width))x\(Int(facts.size.height))"), ("identity", facts.identity), ("audioTracks", facts.audioTracks),
                 ("videoS", String(format: "%.2f", facts.videoSeconds)), ("audioS", String(format: "%.2f", facts.audioSeconds)), ("clipMs", clip.durationMs)])
    try await close(camera)
    XCTAssertLessThanOrEqual(stopToClipMs, 1500)
    XCTAssertGreaterThan(facts.size.height, facts.size.width)
    XCTAssertEqual(facts.audioTracks, 1)
    XCTAssertEqual(facts.videoSeconds, seconds, accuracy: seconds == 5 ? 0.7 : 1.0)
    XCTAssertEqual(facts.audioSeconds, seconds, accuracy: seconds == 5 ? 0.7 : 1.0)
    XCTAssertLessThanOrEqual(abs(facts.audioSeconds - facts.videoSeconds), 0.15)
  }

  @MainActor func test1_open() async throws {
    let (camera, readyMs) = try await open(.circle, .front)
    _ = try await ProbeApp.opening.value
    probe("open", [("layerOpenMs", ProbeApp.openMs), ("readyMs", readyMs)])
    try await close(camera)
    XCTAssertLessThanOrEqual(readyMs, 5000)
  }

  @MainActor func test2_takeFront5s() async throws { try await take("take_front_5s", profile: .circle, lens: .front, seconds: 5) }
  @MainActor func test3_takeFront5sAgain() async throws { try await take("take_front_5s_b", profile: .circle, lens: .front, seconds: 5) }
  @MainActor func test4_takeBack5s() async throws { try await take("take_back_5s", profile: .moment, lens: .back, seconds: 5) }
  @MainActor func test5_takeFront5sAfterMoment() async throws { try await take("take_front_5s_after_moment", profile: .circle, lens: .front, seconds: 5) }
  @MainActor func test6_takeMomentFront5s() async throws { try await take("take_moment_front_5s", profile: .moment, lens: .front, seconds: 5) }
  @MainActor func testA_takeFront15s() async throws { try await take("take_front_15s", profile: .circle, lens: .front, seconds: 15) }

  @MainActor func testB_takeFront60s() async throws { try await take("take_front_60s", profile: .circle, lens: .front, seconds: 60) }

  /// A running session must not hiccup when a take starts: the largest gap between frames around the start stays far under 100 ms.
  @MainActor private func startContinuity(_ name: String, profile: CaptureProfile, lens: CameraLens) async throws {
    let (camera, _) = try await open(profile, lens)
    let times = NSMutableArray()
    let guardLock = NSLock()
    let observation = camera.observeFrames(maxPerSecond: 120) { _ in
      let now = CACurrentMediaTime()
      guardLock.lock(); times.add(now); guardLock.unlock()
    }
    try await Task.sleep(nanoseconds: 1_000_000_000)
    let t0 = CACurrentMediaTime()
    try await camera.startRecording()
    try await Task.sleep(nanoseconds: 2_000_000_000)
    _ = try await camera.stopRecording()
    observation.cancel()
    guardLock.lock()
    let stamps = (times as? [Double] ?? []).filter { $0 >= t0 - 0.5 && $0 <= t0 + 1.5 }
    guardLock.unlock()
    var maxGapMs = 0.0
    for index in stamps.indices.dropFirst() { maxGapMs = max(maxGapMs, (stamps[index] - stamps[index - 1]) * 1000) }
    probe(name, [("maxGapMs", String(format: "%.1f", maxGapMs)), ("frames", stamps.count)])
    try await close(camera)
    XCTAssertLessThan(maxGapMs, 100)
  }

  @MainActor func testC_startContinuity() async throws {
    try await startContinuity("start_front_circle", profile: .circle, lens: .front)
    try await startContinuity("start_back_moment", profile: .moment, lens: .back)
  }

  func test8_noRefusals() {
    let refused = Self.collected.filter { $0.contains(" camera refuse ") }
    for line in refused { print("PROBE refused \(line)") }
    XCTAssertTrue(refused.isEmpty, "a take was refused")
  }

  @MainActor func test9_voice5s() async throws {
    await permissions()
    _ = try await ProbeApp.opening.value
    let recorder = MediaVoiceRecorder()
    let file = Self.folder.appendingPathComponent("voice_5s.m4a")
    try? FileManager.default.removeItem(at: file)
    try await recorder.start(to: file)
    try await Task.sleep(nanoseconds: 5_000_000_000)
    let recording = try await recorder.stop()
    let asset = AVURLAsset(url: recording.url)
    let seconds = try await asset.load(.duration).seconds
    let tracks = try await asset.loadTracks(withMediaType: .audio).count
    probe("voice_5s", [("durationS", String(format: "%.2f", seconds)), ("audioTracks", tracks), ("durationMs", recording.durationMs)])
    XCTAssertTrue(FileManager.default.fileExists(atPath: recording.url.path))
    XCTAssertEqual(seconds, 5, accuracy: 0.7)
    XCTAssertEqual(tracks, 1)
    try? FileManager.default.copyItem(at: recording.url, to: file)
  }
}
