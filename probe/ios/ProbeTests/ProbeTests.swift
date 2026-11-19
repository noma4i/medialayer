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
      facts.size = try await video.load(.naturalSize)
      facts.identity = try await video.load(.preferredTransform) == .identity
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
    XCTAssertTrue(facts.identity, "preferredTransform is not identity")
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
  @MainActor func test3_takeBack5s() async throws { try await take("take_back_5s", profile: .moment, lens: .back, seconds: 5) }
  @MainActor func test4_takeFront15s() async throws { try await take("take_front_15s", profile: .circle, lens: .front, seconds: 15) }

  @MainActor func test8_takeFront60s() async throws { try await take("take_front_60s", profile: .circle, lens: .front, seconds: 60) }

  func test5_traceRules() {
    let lines = Self.collected
    let takes = lines.filter { $0.contains(" camera take first frame ") }
    let outputs = lines.filter { $0.contains(" camera take output ") }
    func number(_ key: String, in line: String) -> Int? {
      guard let range = line.range(of: "\(key)=") else { return nil }
      return Int(line[range.upperBound...].prefix { $0.isNumber })
    }
    let badTakes = takes.filter {
      guard let width = number("width", in: $0), let height = number("height", in: $0) else { return true }
      return height <= width
    }
    let badOutputs = outputs.filter { !$0.contains("transformIdentity=true") }
    var bound = false
    var frameOutputInBind = 0
    for line in lines {
      if line.contains(" camera bind ") { bound = true }
      if line.contains("session commit why=unbind") { bound = false }
      if bound && line.contains("session commit why=frameOutput") { frameOutputInBind += 1 }
    }
    probe("trace_rules", [("takes", takes.count), ("badTakes", badTakes.count), ("outputs", outputs.count), ("badOutputs", badOutputs.count), ("frameOutputInBind", frameOutputInBind)])
    for line in badTakes + badOutputs { print("PROBE bad line \(line)") }
    XCTAssertEqual(takes.count, Self.takeCount)
    XCTAssertTrue(badTakes.isEmpty)
    XCTAssertTrue(badOutputs.isEmpty)
    XCTAssertEqual(frameOutputInBind, 0)
  }

  func test6_coverage() {
    let lines = Self.collected
    let coverage = lines.filter { $0.contains("take coverage") }
    let refused = lines.filter { $0.contains(" camera refuse ") }
    if coverage.isEmpty { probe("coverage", [("coverage", "absent")]) }
    for line in coverage { print("PROBE coverage \(line)") }
    for line in refused { print("PROBE refused \(line)") }
    XCTAssertTrue(refused.isEmpty, "a take was refused")
  }

  @MainActor func test7_voice5s() async throws {
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
