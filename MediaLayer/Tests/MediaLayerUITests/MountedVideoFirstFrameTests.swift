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
  var count = 0
}

private struct Page: View {
  let context: MediaContext
  let frames: FirstFrames

  var body: some View {
    MediaVideo(PageSource(url: "file:///page.mp4"), onFirstFrame: { frames.count += 1 })
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
      try await Task.sleep(for: .milliseconds(200))
      places.emit(.rendered)
      await eventually("the first frame was reported") { frames.count == 1 }

      host.view.isHidden = true
      await eventually("the hidden page withdrew") { places.visibleCalls(of: url) == [true, false] }
      host.view.isHidden = false
      await eventually("the returning page declared itself again") { places.visibleCalls(of: url) == [true, false, true] }
      try await Task.sleep(for: .milliseconds(200))
      places.emit(.loading)
      places.emit(.rendered)
      try await Task.sleep(for: .milliseconds(300))

      #expect(frames.count == 1, "the same source reported its first frame \(frames.count) times across one leave and return")
    }
  }
}
#endif
