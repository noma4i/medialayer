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
  var size: MediaSize? = MediaSize(width: 16, height: 16)
}

private struct ImageColumn: View {
  let count: Int
  let context: MediaContext
  var generation = 0

  var body: some View {
    VStack(spacing: 0) {
      ForEach(0 ..< count, id: \.self) { index in
        MediaImage(PlainImage(url: "https://o.test/main-thread-\(generation)-\(index).png", size: index.isMultiple(of: 2) ? MediaSize(width: 16, height: 16) : nil)).frame(width: 10, height: 10)
      }
    }
    .environment(\.mediaContext, context)
  }
}

private func pngData(_ side: CGFloat) -> Data {
  UIGraphicsImageRenderer(size: CGSize(width: side, height: side)).pngData { $0.fill(CGRect(x: 0, y: 0, width: side, height: side)) }
}

private final class DecodeThreads: @unchecked Sendable {
  private let lock = NSLock()
  private var onMain: [Bool] = []

  func record(_ value: Bool) { lock.withLock { onMain.append(value) } }
  var seen: [Bool] { lock.withLock { onMain } }
}

struct MediaImageMainThreadTests {
  @MainActor
  @Test
  func inv_image_40_imagesCallFfiOnlyForTheCacheKeyOnTheMainThreadAndKeepClaimOrder() async {
    let total = 10
    let places = RecordingPlaces()
    places.imageResult = .success(pngData(16))
    let pixelCalls = DecodeThreads()
    PixelSizeMemo.install(PixelSizeMemo { width, height, scale, declared, crop in
      pixelCalls.record(Thread.isMainThread)
      return imagePixelSize(width: width, height: height, scale: scale, declared: declared, crop: crop)
    }, on: places)
    ImageMemory.install(ImageCache(), on: places)
    let context = MediaContext(places: places)
    _ = context.pipeline
    places.resetMainThreadCalls()
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 400))
    let host = UIHostingController(rootView: ImageColumn(count: 0, context: context))
    window.rootViewController = host
    window.isHidden = false
    defer { window.isHidden = true }
    let urls = (0 ..< total).map { "https://o.test/main-thread-0-\($0).png" }

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

    host.rootView = ImageColumn(count: total, context: context)
    await settle("return ready") { places.recorded.filter { $0 == "state ready" }.count == total * 3 }
    host.rootView = ImageColumn(count: total, context: context, generation: 1)
    await settle("changed ready") { places.recorded.filter { $0 == "state ready" }.count == total * 4 }
    let video = PlainImage(url: "https://o.test/video.mp4")
    let cover = await PlaceholderView.loadCover("https://o.test/cover.png", for: video.item, context: context, points: CGSize(width: 10, height: 10), scale: 1)
    let exported = await context.image(for: PlainImage(url: "https://o.test/export.png"), points: CGSize(width: 10, height: 10), scale: 1)
    places.needsTone = true
    let tone = await ToneFrames(places: places).prepare(source: PlainImage(url: "https://o.test/tone.png").item)
    #expect(cover != nil && exported != nil && tone != nil)
    #expect(places.mainThreadCalls == [])
    #expect(!pixelCalls.seen.isEmpty)
    #expect(!pixelCalls.seen.contains(true))
    for url in urls { #expect(places.visibleCalls(of: url) == [true, false, true, false, true, false]) }
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

  @MainActor
  @Test
  func inv_image_66_anImageDecodesOffTheMainThread() async throws {
    let places = RecordingPlaces()
    places.imageResult = .success(pngData(16))
    let decodedOnMain = DecodeThreads()
    var configuration = ImagePipeline.Configuration(dataLoader: CoreDataLoader(places: places))
    configuration.dataCache = nil
    configuration.isLocalResourcesSupportEnabled = false
    configuration.makeImageDecoder = { context in AnimatedKeepingDecoder(context: context, admit: { _ in decodedOnMain.record(Thread.isMainThread); return {} }) }
    let pipeline = ImagePipeline(configuration: configuration)
    let request = try #require(MediaImagePipeline.request(places: places, for: PlainImage(url: "https://o.test/decode-thread.png"), points: CGSize(width: 16, height: 16), contentMode: .fill))

    _ = try await pipeline.image(for: request)

    #expect(decodedOnMain.seen == [false])
  }
}
#endif
