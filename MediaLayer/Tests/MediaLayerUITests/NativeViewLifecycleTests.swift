#if canImport(UIKit)
import AVFoundation
import MediaLayer
import SwiftUI
import Testing
import UIKit
@testable import MediaLayerUI

private struct Source: MediaSource {
  let url: String
  let kind: Kind
  var headers: [String: String] { [:] }
}

@MainActor
private final class Inputs: ObservableObject {
  @Published var url = "first"
  var audio: AudioPlayback?
  var reports = 0
}

private struct Cell: View {
  @ObservedObject var inputs: Inputs
  let context: MediaContext
  let audio: Bool

  var body: some View {
    Group {
      if audio {
        MediaAudio(Source(url: inputs.url, kind: .audio), playing: true) { playback in
          Color.clear.onAppear { inputs.audio = playback }
        }
      } else {
        MediaVideo(Source(url: inputs.url, kind: .video), onProgress: { _ in inputs.reports += 1 })
      }
    }
    .environment(\.mediaContext, context)
    .frame(width: 80, height: 80)
  }
}

@MainActor
private final class Screen {
  let inputs = Inputs()
  let places = RecordingPlaces()
  let slots = LoadedSlots()
  let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 200))
  let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 200, height: 200))
  let host: UIHostingController<Cell>

  init(audio: Bool = false) {
    host = UIHostingController(rootView: Cell(inputs: inputs, context: MediaContext(places: places, slots: slots), audio: audio))
    let page = UIViewController()
    window.rootViewController = page
    page.addChild(host)
    scroll.contentSize = CGSize(width: 200, height: 1000)
    scroll.contentInsetAdjustmentBehavior = .never
    page.view.addSubview(scroll)
    host.view.frame = CGRect(x: 0, y: 0, width: 80, height: 80)
    host.safeAreaRegions = []
    scroll.addSubview(host.view)
    host.didMove(toParent: page)
    window.isHidden = false
  }

  func close() {
    window.isHidden = true
    window.rootViewController = nil
  }

  func settle(_ duration: Duration = .milliseconds(350)) async {
    try? await Task.sleep(for: duration)
  }
}

extension GlobalMedia {
  @MainActor
  @Suite(.serialized, .timeLimit(.minutes(2)))
  struct NativeViewLifecycleTests {
    @Test
    func inv_native_05_audio_progress_stops_when_inactive_and_resumes() async throws {
      let screen = Screen(audio: true)
      defer { screen.close() }
      let slot = AVQueuePlayerSlotFactory(startMuted: true, slots: screen.slots).create(slot: 0) as! AVQueuePlayerSlot
      let clip = Bundle.module.url(forResource: "silent", withExtension: "mp4", subdirectory: "Fixtures")!.absoluteString
      slot.load(key: "first", route: PlaybackRoute(url: clip, headers: [:], generation: nil, hls: false, looping: false))
      defer { slot.unload() }
      for _ in 0..<100 {
        if slot.sample().durationMs != nil { break }
        await screen.settle(.milliseconds(50))
      }
      let duration = try #require(slot.sample().durationMs)
      let audio = try #require(screen.inputs.audio)
      await slot.player.seek(to: CMTime(value: Int64(duration / 5), timescale: 1000), toleranceBefore: .zero, toleranceAfter: .zero)
      await screen.settle()
      #expect(audio.progress > 0)
      NotificationCenter.default.post(name: UIApplication.didEnterBackgroundNotification, object: nil)
      await screen.settle()
      let background = audio.progress
      await slot.player.seek(to: CMTime(value: Int64(duration * 2 / 5), timescale: 1000), toleranceBefore: .zero, toleranceAfter: .zero)
      await screen.settle(.milliseconds(600))
      #expect(audio.progress == background)
      NotificationCenter.default.post(name: UIApplication.willEnterForegroundNotification, object: nil)
      await screen.settle()
      #expect(audio.progress > background)
      screen.scroll.contentOffset = CGPoint(x: 0, y: 500)
      await screen.settle()
      let offscreen = audio.progress
      await slot.player.seek(to: CMTime(value: Int64(duration * 3 / 5), timescale: 1000), toleranceBefore: .zero, toleranceAfter: .zero)
      await screen.settle(.milliseconds(600))
      #expect(audio.progress == offscreen)
      screen.scroll.contentOffset = .zero
      await screen.settle()
      #expect(audio.progress > offscreen)
    }

    @Test
    func inv_native_02_video_defaults_to_playing() async {
      let screen = Screen()
      defer { screen.close() }
      await screen.settle()
      #expect(screen.places.recorded.contains("playing true"))
    }

    @Test
    func inv_native_03_video_url_replacement_releases_claim_and_attachment() async {
      let screen = Screen()
      defer { screen.close() }
      await screen.settle()
      #expect(screen.places.recorded.contains("attach first"))
      screen.inputs.url = "second"
      await screen.settle()
      #expect(screen.places.visibleCalls(of: "first") == [true, false])
      #expect(screen.places.visibleCalls(of: "second") == [true])
      #expect(screen.places.recorded.contains("detach first"))
      #expect(screen.places.recorded.contains("attach second"))
      screen.close()
      await screen.settle()
      #expect(screen.places.visibleCalls(of: "second") == [true, false])
    }

    @Test
    func inv_native_03_audio_url_replacement_replaces_playback_and_claim() async throws {
      let screen = Screen(audio: true)
      defer { screen.close() }
      await screen.settle()
      let old = try #require(screen.inputs.audio)
      screen.inputs.url = "second"
      await screen.settle()
      let current = try #require(screen.inputs.audio)
      #expect(current !== old)
      #expect(screen.places.visibleCalls(of: "first") == [true, false])
      #expect(screen.places.visibleCalls(of: "second") == [true])
      screen.close()
      await screen.settle()
      #expect(screen.places.visibleCalls(of: "second") == [true, false])
    }

    @Test
    func inv_native_05_video_progress_stops_when_inactive_and_resumes() async throws {
      let screen = Screen()
      defer { screen.close() }
      let slot = AVQueuePlayerSlotFactory(startMuted: true, slots: screen.slots).create(slot: 0) as! AVQueuePlayerSlot
      let clip = Bundle.module.url(forResource: "silent", withExtension: "mp4", subdirectory: "Fixtures")!.absoluteString
      slot.load(key: "first", route: PlaybackRoute(url: clip, headers: [:], generation: nil, hls: false, looping: true))
      defer { slot.unload() }
      for _ in 0..<100 {
        if screen.inputs.reports >= 2 { break }
        await screen.settle(.milliseconds(50))
      }
      #expect(screen.inputs.reports >= 2)
      NotificationCenter.default.post(name: UIApplication.didEnterBackgroundNotification, object: nil)
      await screen.settle()
      let background = screen.inputs.reports
      await screen.settle()
      #expect(screen.inputs.reports == background)
      NotificationCenter.default.post(name: UIApplication.willEnterForegroundNotification, object: nil)
      await screen.settle()
      #expect(screen.inputs.reports > background)
      screen.scroll.contentOffset = CGPoint(x: 0, y: 500)
      await screen.settle()
      let offscreen = screen.inputs.reports
      await screen.settle()
      #expect(screen.inputs.reports == offscreen)
      screen.scroll.contentOffset = .zero
      await screen.settle()
      #expect(screen.inputs.reports > offscreen)
    }
  }
}
#endif
