#if canImport(UIKit)
import MediaLayer
import SwiftUI
import Testing
import UIKit
@testable import MediaLayerUI

private struct PageSource: MediaSource {
  let url: String
  var kind: Kind { .video }
  var headers: [String: String] { [:] }
}

@MainActor
private final class FirstFrames {
  var count = 0 { didSet { TestSignal.shared.fire() } }
  let states = AsyncStream<MediaState>.makeStream()
}

private struct Page: View {
  let context: MediaContext
  let frames: FirstFrames

  var body: some View {
    MediaVideo(PageSource(url: "file:///page.mp4"), onFirstFrame: { frames.count += 1 }) { state in
      Color.clear.onChange(of: state) { _, value in frames.states.continuation.yield(value); TestSignal.shared.fire() }
    }
      .environment(\.mediaContext, context)
      .frame(width: 80, height: 80)
  }
}

extension GlobalMedia {
  @MainActor
  @Suite(.serialized, .timeLimit(.minutes(2)))
  struct MountedVideoFirstFrameTests {
    @Test
    func inv_mount_02_aPageThatLeavesAndReturnsKeepsItsFirstFrameFlag() async throws {
      let places = RecordingPlaces()
      let frames = FirstFrames()
      var subscriptions = places.stateSubscriptionEvents.stream.makeAsyncIterator()
      var states = frames.states.stream.makeAsyncIterator()
      let host = UIHostingController(rootView: Page(context: MediaContext(places: places, slots: LoadedSlots()), frames: frames))
      let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 200))
      let root = UIViewController()
      window.rootViewController = root
      root.addChild(host)
      host.view.frame = CGRect(x: 0, y: 0, width: 80, height: 80)
      host.safeAreaRegions = []
      root.view.addSubview(host.view)
      host.didMove(toParent: root)
      window.isHidden = false
      defer {
        window.isHidden = true
        window.rootViewController = nil
      }
      let url = "file:///page.mp4"

      await eventually("the page declared itself visible") { places.visibleCalls(of: url) == [true] }
      _ = await subscriptions.next()
      places.emit(.rendered)
      await eventually("the first frame was reported") { frames.count == 1 }
      while await states.next() != .rendered {}

      host.view.isHidden = true
      await eventually("the hidden page withdrew") { places.visibleCalls(of: url) == [true, false] }
      host.view.isHidden = false
      await eventually("the returning page declared itself again") { places.visibleCalls(of: url) == [true, false, true] }
      _ = await subscriptions.next()
      places.emit(.loading)
      while await states.next() != .loading {}
      places.emit(.rendered)
      while await states.next() != .rendered {}

      #expect(frames.count == 1, "the same source reported its first frame \(frames.count) times across one leave and return")
    }

    @Test
    func inv_placeholder_10_aReadyPageKeepsItsCoverAndAReturnAfterTheRenderedFrameNeverAsksForItAgain() async throws {
      let places = RecordingPlaces()
      let frames = FirstFrames()
      var subscriptions = places.stateSubscriptionEvents.stream.makeAsyncIterator()
      var states = frames.states.stream.makeAsyncIterator()
      let host = UIHostingController(rootView: Page(context: MediaContext(places: places, slots: LoadedSlots()), frames: frames))
      let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 200))
      let root = UIViewController()
      window.rootViewController = root
      root.addChild(host)
      host.view.frame = CGRect(x: 0, y: 0, width: 80, height: 80)
      host.safeAreaRegions = []
      root.view.addSubview(host.view)
      host.didMove(toParent: root)
      window.isHidden = false
      defer {
        window.isHidden = true
        window.rootViewController = nil
      }
      let url = "file:///page.mp4"

      await eventually("the page asked for its cover") { !places.placeholderReadyAsks.isEmpty }
      _ = await subscriptions.next()
      places.emit(.loading)
      while await states.next() != .loading {}
      places.emit(.ready)
      while await states.next() != .ready {}
      let beforeFrame = places.placeholderReadyAsks
      places.emit(.rendered)
      await eventually("the rendered frame released the cover") { places.placeholderReadyAsks.last == true }

      host.view.isHidden = true
      await eventually("the hidden page withdrew") { places.visibleCalls(of: url) == [true, false] }
      host.view.isHidden = false
      await eventually("the returning page declared itself again") { places.visibleCalls(of: url) == [true, false, true] }
      _ = await subscriptions.next()
      places.emit(.idle)
      while await states.next() != .idle {}
      places.emit(.loading)
      while await states.next() != .loading {}

      let asks = places.placeholderReadyAsks
      #expect(beforeFrame.allSatisfy { !$0 }, "a ready player released the cover before its first frame: \(beforeFrame)")
      #expect(asks.drop(while: { !$0 }).allSatisfy { $0 }, "the cover was asked for again after its frame was shown: \(asks)")
    }
  }
}
#endif
