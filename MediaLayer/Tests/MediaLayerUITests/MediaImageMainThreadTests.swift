#if canImport(UIKit)
import MediaLayer
import Nuke
import SwiftUI
import Testing
import UIKit
@testable import MediaLayerUI

private struct PlainImage: MediaSource {
  let url: String
  var kind: Kind { .image }
  var headers: [String: String] { [:] }
  var size: MediaSize? { MediaSize(width: 16, height: 16) }
}

private struct ImageColumn: View {
  let count: Int
  let context: MediaContext

  var body: some View {
    VStack(spacing: 0) {
      ForEach(0 ..< count, id: \.self) { index in
        MediaImage(PlainImage(url: "https://o.test/main-thread-\(index).png")).frame(width: 10, height: 10)
      }
    }
    .environment(\.mediaContext, context)
  }
}

struct MediaImageMainThreadTests {
  @MainActor
  @Test
  func inv_image_39_imagesCallCoreStatsNeverAndCacheKeyOncePerImageOnTheMainThread() async {
    let places = RecordingPlaces()
    ImageMemory.install(ImageCache(), on: places)
    let context = MediaContext(places: places)
    _ = context.pipeline
    places.resetMainThreadImageStats()
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 400))
    let host = UIHostingController(rootView: ImageColumn(count: 20, context: context))
    window.rootViewController = host
    window.isHidden = false
    defer { window.isHidden = true }

    let end = Date().addingTimeInterval(1)
    while Date() < end {
      RunLoop.main.run(until: Date().addingTimeInterval(0.02))
      await Task.yield()
    }

    #expect(places.mainThreadImageStats == 0)
    #expect(places.cacheKeyReads <= 20)
  }
}
#endif
