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

private final class KeyReads: @unchecked Sendable {
  private let lock = NSLock()
  private var seen: [(String, Bool)] = []

  func record(_ url: String, onMain: Bool) { lock.withLock { seen.append((url, onMain)) } }
  var urls: [String] { lock.withLock { seen.map(\.0) } }
  var onMain: [String] { lock.withLock { seen.filter(\.1).map(\.0) } }
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
  let side: CGFloat

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
    .frame(width: side, height: side)
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

  init(audio: Bool = false, side: CGFloat = 80) {
    SourceKey.read = { $0.url }
    host = UIHostingController(rootView: Cell(inputs: inputs, context: MediaContext(places: places, slots: slots), audio: audio, side: side))
    let page = UIViewController()
    window.rootViewController = page
    page.addChild(host)
    scroll.contentSize = CGSize(width: 200, height: 1000)
    scroll.contentInsetAdjustmentBehavior = .never
    page.view.addSubview(scroll)
    host.view.frame = CGRect(x: 0, y: 0, width: side, height: side)
    host.safeAreaRegions = []
    scroll.addSubview(host.view)
    host.didMove(toParent: page)
    window.isHidden = false
  }

  func close() {
    SourceKey.read = SourceKey.core
    window.isHidden = true
    window.rootViewController = nil
  }

  func settle(_ duration: Duration = .milliseconds(350)) async {
    try? await Task.sleep(for: duration)
  }

  func settle(until holds: () -> Bool) async {
    for _ in 0..<200 where !holds() { try? await Task.sleep(for: .milliseconds(50)) }
  }
}

extension GlobalMedia {
  @MainActor
  @Suite(.serialized, .timeLimit(.minutes(2)))
  struct NativeViewLifecycleTests {
    @Test
    func inv_idle_05_two_consumers_share_one_progress_sample_and_release_it() async throws {
      let slots = LoadedSlots()
      let slot = await AVQueuePlayerSlotFactory(startMuted: true, slots: slots).makeSlot(0)
      let clip = Bundle.module.url(forResource: "silent", withExtension: "mp4", subdirectory: "Fixtures")!.absoluteString
      await slot.load(key: "shared", route: PlaybackRoute(url: clip, headers: [:], generation: nil, hls: false, looping: false))
      defer { slot.close() }
      await eventually("slot assigned") { slots.slot(for: "shared") === slot }
      let first = slots.progress.stream(for: "shared")
      let second = slots.progress.stream(for: "shared")
      let a = Task { for await _ in first {} }
      let b = Task { for await _ in second {} }
      #expect(slots.progress.producerCount == 1)
      let before = slots.progress.samples
      slots.progress.changed("shared")
      #expect(slots.progress.samples - before == 1)
      a.cancel()
      b.cancel()
      await a.value
      await b.value
      await Task.yield()
      #expect(slots.progress.producerCount == 0)
    }

    @Test
    func inv_idle_05_paused_video_does_not_repeat_progress_and_seek_resumes_it() async throws {
      let screen = Screen()
      defer { screen.close() }
      let slot = await AVQueuePlayerSlotFactory(startMuted: true, slots: screen.slots).makeSlot(0)
      let clip = Bundle.module.url(forResource: "silent", withExtension: "mp4", subdirectory: "Fixtures")!.absoluteString
      await slot.load(key: "first", route: PlaybackRoute(url: clip, headers: [:], generation: nil, hls: false, looping: false))
      defer { slot.close() }
      for _ in 0..<100 {
        if screen.inputs.reports > 0 { break }
        await screen.settle(.milliseconds(50))
      }
      #expect(screen.inputs.reports > 0)
      #expect(screen.slots.progress.periodicObserverCount == 0)
      let reports = screen.inputs.reports
      await screen.settle(.milliseconds(450))
      #expect(screen.inputs.reports == reports)
      await slot.seek(positionMs: 200)
      await screen.settle(until: { screen.inputs.reports > reports })
      #expect(screen.inputs.reports > reports)
    }

    @Test
    func inv_native_05_audio_progress_stops_in_background_and_resumes() async throws {
      let screen = Screen(audio: true)
      defer { screen.close() }
      let slot = await AVQueuePlayerSlotFactory(startMuted: true, slots: screen.slots).makeSlot(0)
      let clip = Bundle.module.url(forResource: "silent", withExtension: "mp4", subdirectory: "Fixtures")!.absoluteString
      await slot.load(key: "first", route: PlaybackRoute(url: clip, headers: [:], generation: nil, hls: false, looping: false))
      defer { slot.close() }
      for _ in 0..<100 {
        if slot.sample().durationMs != nil { break }
        await screen.settle(.milliseconds(50))
      }
      let duration = try #require(slot.sample().durationMs)
      let audio = try #require(screen.inputs.audio)
      await slot.seek(positionMs: duration / 5)
      await screen.settle(until: { audio.progress > 0 })
      #expect(audio.progress > 0)
      NotificationCenter.default.post(name: UIApplication.didEnterBackgroundNotification, object: nil)
      await screen.settle()
      let background = audio.progress
      await slot.seek(positionMs: duration * 2 / 5)
      await screen.settle(.milliseconds(600))
      #expect(audio.progress == background)
      NotificationCenter.default.post(name: UIApplication.willEnterForegroundNotification, object: nil)
      await screen.settle(until: { audio.progress > background })
      #expect(audio.progress > background)
      screen.scroll.contentOffset = CGPoint(x: 0, y: 500)
      await screen.settle()
      let offscreen = audio.progress
      await slot.seek(positionMs: duration * 3 / 5)
      await screen.settle(until: { audio.progress > offscreen })
      #expect(audio.progress > offscreen)
    }

    @Test
    func inv_audio_02_progressSeekAndEndedIgnoreTheHostFrame() async throws {
      let screen = Screen(audio: true, side: 0)
      defer { screen.close() }
      let slot = await AVQueuePlayerSlotFactory(startMuted: true, slots: screen.slots).makeSlot(0)
      let clip = Bundle.module.url(forResource: "silent", withExtension: "mp4", subdirectory: "Fixtures")!.absoluteString
      await slot.load(key: "first", route: PlaybackRoute(url: clip, headers: [:], generation: nil, hls: false, looping: false))
      defer { slot.close() }
      for _ in 0..<100 {
        if slot.sample().durationMs != nil { break }
        await screen.settle(.milliseconds(50))
      }
      let duration = try #require(slot.sample().durationMs)
      let audio = try #require(screen.inputs.audio)
      await slot.seek(positionMs: duration / 5)
      await screen.settle(until: { audio.progress > 0 })
      #expect(audio.progress > 0)
      audio.seek(to: 0.5)
      await screen.settle()
      #expect(screen.places.recorded.contains("seek 0.5"))
      screen.places.emit(.ended)
      await screen.settle()
      #expect(audio.state == .ended)
      #expect(!audio.isPlaying)
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
      #expect(screen.places.recorded.contains("release first"))
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
      let slot = await AVQueuePlayerSlotFactory(startMuted: true, slots: screen.slots).makeSlot(0)
      let clip = Bundle.module.url(forResource: "silent", withExtension: "mp4", subdirectory: "Fixtures")!.absoluteString
      await slot.load(key: "first", route: PlaybackRoute(url: clip, headers: [:], generation: nil, hls: false, looping: true))
      await slot.play()
      defer { slot.close() }
      for _ in 0..<100 {
        if screen.inputs.reports >= 2 { break }
        await screen.settle(.milliseconds(50))
      }
      #expect(screen.inputs.reports >= 2)
      #expect(screen.slots.progress.periodicObserverCount == 1)
      NotificationCenter.default.post(name: UIApplication.didEnterBackgroundNotification, object: nil)
      await screen.settle()
      let background = screen.inputs.reports
      #expect(screen.slots.progress.periodicObserverCount == 0)
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
    @Test
    func inv_source_11_aPlaceDeclaredInAnyAcceptedFormFollowsTheProgressOfTheCoreKeyAndReadsItOffTheMainThread() async throws {
      let forms = [(" https://o.test/a.mp4 ", "https://o.test/a.mp4"), ("/tmp/a b.mp4", "file:///tmp/a%20b.mp4")]
      let reads = KeyReads()
      let screen = Screen()
      defer { screen.close() }
      SourceKey.read = { item in reads.record(item.url, onMain: Thread.isMainThread); return forms.first { $0.0 == item.url }?.1 ?? item.url }
      let clip = Bundle.module.url(forResource: "silent", withExtension: "mp4", subdirectory: "Fixtures")!.absoluteString
      let factory = AVQueuePlayerSlotFactory(startMuted: true, slots: screen.slots)
      let first = await factory.makeSlot(0)
      let second = await factory.makeSlot(1)
      defer { first.close(); second.close() }
      let route = PlaybackRoute(url: clip, headers: [:], generation: nil, hls: false, looping: false)
      await first.load(key: forms[0].1, route: route)
      await second.load(key: forms[1].1, route: route)

      for (raw, _) in forms {
        screen.inputs.reports = 0
        screen.inputs.url = raw
        await screen.settle(until: { screen.inputs.reports > 0 })
        #expect(screen.inputs.reports > 0, "\(raw)")
      }
      #expect(reads.urls.contains(forms[0].0) && reads.urls.contains(forms[1].0))
      #expect(reads.onMain == [])
    }
  }
}
#endif
