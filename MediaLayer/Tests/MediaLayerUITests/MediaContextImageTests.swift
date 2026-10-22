import CoreGraphics
import Foundation
import MediaLayer
import Nuke
import Testing
@testable import MediaLayerUI

private struct ExportSource: MediaSource {
  let url: String
  var kind: Kind { .image }
  var headers: [String: String] = [:]
  var size: MediaSize?
}

private func pixelWidth(_ image: PlatformImage?) -> Int? {
  #if canImport(UIKit)
  image?.cgImage?.width
  #else
  image?.cgImage(forProposedRect: nil, context: nil, hints: nil)?.width
  #endif
}

private func makeContext(_ places: RecordingPlaces) -> MediaContext {
  ImageMemory.install(ImageCache(), on: places)
  return MediaContext(places: places)
}

extension GlobalMedia {
  @Suite(.timeLimit(.minutes(1)))
  struct MediaContextImageTests {
    @Test
    func theCallReturnsTheDecodedImageOfALoadableSourceWithoutAView() async {
      let places = RecordingPlaces()
      places.imageResult = .success(pngData(side: 16))
      let context = makeContext(places)

      let image = await context.image(for: ExportSource(url: "https://o.test/a.png"), points: CGSize(width: 8, height: 8), scale: 1)

      #expect(pixelWidth(image) == 8)
    }

    @Test
    func inv_image_37_theExportNeverChangesTheStateOfAPlace() async {
      let places = RecordingPlaces()
      places.imageResult = .success(pngData(side: 16))
      let context = makeContext(places)
      let source = ExportSource(url: "https://o.test/e.png")
      _ = await context.image(for: source, points: CGSize(width: 8, height: 8), scale: 1)
      _ = await context.image(for: source, points: CGSize(width: 8, height: 8), scale: 1)
      places.imageResult = .failure(URLError(.notConnectedToInternet))
      _ = await context.image(for: ExportSource(url: "https://o.test/bad.png"), points: CGSize(width: 8, height: 8), scale: 1)

      #expect(places.recorded.filter { $0.hasPrefix("state") }.isEmpty)
    }

    @Test
    func theCallReturnsNilWhenTheSourceFails() async {
      let places = RecordingPlaces()
      places.imageResult = .failure(URLError(.notConnectedToInternet))
      let context = makeContext(places)

      let image = await context.image(for: ExportSource(url: "https://o.test/bad.png"), points: CGSize(width: 8, height: 8), scale: 1)

      #expect(image == nil)
    }

    @Test(arguments: [false, true])
    func theCallReturnsNilAndWritesNoStateWhenTheTaskIsCancelled(cached: Bool) async {
      let places = RecordingPlaces()
      places.imageResult = .success(pngData(side: 16))
      let context = makeContext(places)
      let source = ExportSource(url: "https://o.test/c.png")
      let points = CGSize(width: 8, height: 8)
      if cached { _ = await context.image(for: source, points: points, scale: 1) }
      let before = (places.recorded, places.fetchedURLs)
      let task = Task {
        while !Task.isCancelled { await Task.yield() }
        return await context.image(for: source, points: points, scale: 1)
      }

      task.cancel()
      let image = await task.value

      #expect(image == nil)
      #expect(places.recorded == before.0)
      #expect(places.fetchedURLs == before.1)
    }

    @Test
    func aRepeatedCallIsServedFromTheCacheWithNoSecondFetch() async {
      let places = RecordingPlaces()
      places.imageResult = .success(pngData(side: 16))
      let context = makeContext(places)
      let source = ExportSource(url: "https://o.test/d.png")

      let first = await context.image(for: source, points: CGSize(width: 8, height: 8), scale: 1)
      let second = await context.image(for: source, points: CGSize(width: 8, height: 8), scale: 1)

      #expect(first != nil && second != nil)
      #expect(places.fetchedURLs == ["https://o.test/d.png"])
    }

    @Test
    func aFileSourceLoadsThroughTheCoreWithoutTheProxy() async throws {
      let file = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).png")
      try pngData(side: 16).write(to: file)
      let media = try MediaLayer.open(
        fetcher: URLSessionFetcher(),
        settings: UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!),
        directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
      )

      let image = await MediaContext(places: media).image(for: ExportSource(url: file.absoluteString), points: CGSize(width: 8, height: 8), scale: 1)

      #expect(pixelWidth(image) == 8)
    }
  }
}
