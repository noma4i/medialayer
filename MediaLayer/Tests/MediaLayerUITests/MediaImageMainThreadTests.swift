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

private func pngData(_ side: CGFloat) -> Data {
  UIGraphicsImageRenderer(size: CGSize(width: side, height: side)).pngData { $0.fill(CGRect(x: 0, y: 0, width: side, height: side)) }
}

struct MediaImageMainThreadTests {
  @MainActor
  @Test
  func inv_image_40_imagesCallFfiOnlyForTheCacheKeyOnTheMainThreadAndKeepClaimOrder() async {
    let total = 10
    let places = RecordingPlaces()
    places.imageResult = .success(pngData(16))
    ImageMemory.install(ImageCache(), on: places)
    let context = MediaContext(places: places)
    _ = context.pipeline
    places.resetMainThreadCalls()
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 400))
    let host = UIHostingController(rootView: ImageColumn(count: 0, context: context))
    window.rootViewController = host
    window.isHidden = false
    defer { window.isHidden = true }
    let urls = (0 ..< total).map { "https://o.test/main-thread-\($0).png" }

    func settle(_ condition: String, _ holds: () -> Bool) async {
      let end = Date().addingTimeInterval(5)
      while !holds() {
        guard Date() < end else { Issue.record("never became true: \(condition)"); return }
        RunLoop.main.run(until: Date().addingTimeInterval(0.02))
        await Task.yield()
      }
    }

    for round in 1 ... 2 {
      host.rootView = ImageColumn(count: total, context: context)
      await settle("visible \(round)") { urls.allSatisfy { places.visibleCalls(of: $0).filter { $0 }.count == round } }
      await settle("ready \(round)") { places.recorded.filter { $0 == "state ready" }.count == total * round }
      host.rootView = ImageColumn(count: 0, context: context)
      await settle("released \(round)") { urls.allSatisfy { places.visibleCalls(of: $0).filter { !$0 }.count == round } }
    }

    #expect(places.mainThreadCalls.filter { $0 != "cacheKey" } == [])
    #expect(places.mainThreadCalls.count <= total * 2)
    for url in urls { #expect(places.visibleCalls(of: url) == [true, false, true, false]) }
  }

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
