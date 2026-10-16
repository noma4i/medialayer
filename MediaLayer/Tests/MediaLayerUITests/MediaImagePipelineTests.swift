import CoreGraphics
import Foundation
import ImageIO
import MediaLayer
import Nuke
import SwiftUI
import Testing
import UniformTypeIdentifiers
@testable import MediaLayerUI

private struct TestImage: MediaSource {
  let url: String
  var kind: Kind { .image }
  var headers: [String: String] = [:]
  var size: MediaSize?
}

private func resizeIdentifier(_ request: ImageRequest?) -> String {
  request?.processors.first?.identifier ?? ""
}

private func centerPixel(_ image: CGImage?) -> [UInt8]? {
  guard let image else { return nil }
  var pixel = [UInt8](repeating: 0, count: 4)
  let drawn = pixel.withUnsafeMutableBytes { bytes -> Bool in
    guard let context = CGContext(
      data: bytes.baseAddress, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ) else { return false }
    context.draw(image, in: CGRect(x: -image.width / 2, y: -image.height / 2, width: image.width, height: image.height))
    return true
  }
  return drawn ? pixel : nil
}

private func pixelWidth(_ image: PlatformImage) -> Int? {
  #if canImport(UIKit)
  image.cgImage?.width
  #else
  image.cgImage(forProposedRect: nil, context: nil, hints: nil)?.width
  #endif
}

extension GlobalMedia {
  @Suite(.timeLimit(.minutes(1)))
  struct MediaImagePipelineTests {
    @Test(arguments: [(MediaSize?.some(MediaSize(width: 8, height: 8)), 1), (MediaSize?.none, 0)])
    func requestResizesToTheDeclaredSizeAndForwardsHeaders(size: MediaSize?, processors: Int) throws {
      let request = try #require(MediaImagePipeline.request(for: TestImage(url: "https://o.test/a.png", headers: ["X-Token": "t"], size: size)))

      #expect(request.processors.count == processors)
      #expect(request.urlRequest?.value(forHTTPHeaderField: "X-Token") == "t")
    }

    @Test(arguments: [
      (CGSize(width: 50, height: 50), CGFloat(3), MediaSize?.none, "(152.0, 152.0)"),
      (CGSize(width: 100.4, height: 20), CGFloat(2), MediaSize?.none, "(208.0, 40.0)"),
      (CGSize(width: 50, height: 50), CGFloat(3), MediaSize?.some(MediaSize(width: 16, height: 16)), "(16.0, 16.0)"),
      (CGSize(width: 0, height: 0), CGFloat(3), MediaSize?.some(MediaSize(width: 16, height: 16)), "(16.0, 16.0)")
    ])
    func inv_image_01_the_request_decodes_at_the_view_size_in_pixels_rounded_up_to_8_and_a_declared_size_wins(
      points: CGSize, scale: CGFloat, declared: MediaSize?, expected: String
    ) throws {
      let request = MediaImagePipeline.request(for: TestImage(url: "https://o.test/a.png", size: declared), points: points, scale: scale)

      #expect(resizeIdentifier(request).contains("s=\(expected)"))
    }

    @Test
    func inv_image_01_a_view_without_size_or_a_declared_size_has_nothing_to_decode_at() throws {
      let request = try #require(MediaImagePipeline.request(for: TestImage(url: "https://o.test/a.png"), points: .zero, scale: 3))

      #expect(request.processors.isEmpty)
    }

    @Test
    func inv_image_01_the_core_is_asked_for_the_plain_url_whatever_the_decode_size() async throws {
      let places = RecordingPlaces()
      places.imageResult = .success(pngData(side: 16))
      ImageMemory.install(ImageCache(), on: places)
      let pipeline = MediaImagePipeline.make(places: places)
      let source = TestImage(url: "https://o.test/a.png")

      for side in [CGFloat(50), 80] {
        _ = try await pipeline.image(for: try #require(MediaImagePipeline.request(for: source, points: CGSize(width: side, height: side), scale: 2)))
      }

      #expect(places.fetchedURLs == ["https://o.test/a.png", "https://o.test/a.png"])
    }

    @Test
    func inv_image_04_the_content_mode_picks_the_resize_mode_and_keeps_the_decode_size() throws {
      let source = TestImage(url: "https://o.test/a.png")
      let fill = resizeIdentifier(MediaImagePipeline.request(for: source, points: CGSize(width: 50, height: 50), scale: 3, contentMode: .fill))
      let fit = resizeIdentifier(MediaImagePipeline.request(for: source, points: CGSize(width: 50, height: 50), scale: 3, contentMode: .fit))

      #expect(fill.contains("cm=.aspectFill") && fill.contains("s=(152.0, 152.0)"))
      #expect(fit.contains("cm=.aspectFit") && fit.contains("s=(152.0, 152.0)"))
    }

    @MainActor
    @Test
    func inv_image_05_an_image_in_the_decoder_memory_shows_in_the_first_frame() async throws {
      let places = RecordingPlaces()
      places.imageResult = .success(pngData(side: 16))
      ImageMemory.install(ImageCache(), on: places)
      let context = MediaContext(places: places)
      let source = TestImage(url: "https://o.test/shown.png")
      func shownFirstFrame() -> [UInt8]? {
        let view = MediaImage(source).environment(\.mediaContext, context).environment(\.displayScale, 2).frame(width: 50, height: 50)
        return centerPixel(firstFrame(of: view, size: CGSize(width: 50, height: 50)))
      }
      let request = try #require(MediaImagePipeline.request(for: source, points: CGSize(width: 50, height: 50), scale: 2))

      let held = Latch()
      places.fetchLatch = held
      let cold = shownFirstFrame()
      held.open()
      _ = try await context.pipeline.image(for: request)
      let warm = shownFirstFrame()

      let larger = try #require(MediaImagePipeline.request(for: source, points: CGSize(width: 80, height: 80), scale: 2))
      #expect(MediaImagePipeline.cachedImage(for: request, in: context.pipeline) != nil)
      #expect(MediaImagePipeline.cachedImage(for: larger, in: context.pipeline) == nil)
      #expect(cold?.last == 0)
      #expect(warm?.first.map { $0 > 200 } == true && warm?.last == 255)
    }

    @Test
    func inv_memory_10_the_decoder_memory_is_limited_by_the_open_option_and_a_non_positive_limit_is_refused() throws {
      let settings = UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!)
      let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)

      let limited = try MediaLayer.open(fetcher: URLSessionFetcher(), settings: settings, directory: directory, imageMemoryBytes: 5_000_000)
      let defaulted = try MediaLayer.open(fetcher: URLSessionFetcher(), settings: settings, directory: directory.appendingPathComponent("b"))

      #expect(ImageMemory.cache(for: limited).costLimit == 5_000_000)
      #expect(ImageMemory.cache(for: defaulted).costLimit == 128 * 1024 * 1024)
      #expect(throws: MediaError.self) {
        try MediaLayer.open(fetcher: URLSessionFetcher(), settings: settings, directory: directory.appendingPathComponent("c"), imageMemoryBytes: 0)
      }
    }

    @Test
    func inv_image_03_clearing_the_cache_drops_the_decoder_memory() async throws {
      let media = try MediaLayer.open(
        fetcher: URLSessionFetcher(),
        settings: UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!),
        directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
      )
      let pipeline = MediaImagePipeline.make(places: media)
      let request = try #require(MediaImagePipeline.request(for: TestImage(url: "https://o.test/a.png"), points: CGSize(width: 50, height: 50), scale: 2))
      pipeline.cache[request] = ImageContainer(image: try ImageDecoders.Default().decode(pngData(side: 4)).image)
      #expect(pipeline.cache[request] != nil)

      await media.clear(kind: .video)
      #expect(pipeline.cache[request] != nil)
      await media.clear(kind: nil)

      #expect(pipeline.cache[request] == nil)
    }

    @Test
    func imageLoadsThroughTheCoreAndASecondPipelineReadsTheCoreDiskCacheWithTheOriginDown() async throws {
      let server = try LocalHTTPServer()
      try await server.start()
      server.serve(pngData(side: 16), at: "/image.png")
      let media = try MediaLayer.open(
        fetcher: URLSessionFetcher(),
        settings: UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!),
        directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
      )
      let source = TestImage(url: server.url("/image.png"), headers: ["Authorization": "Bearer t"], size: MediaSize(width: 4, height: 4))
      let request = try #require(MediaImagePipeline.request(for: source))

      let first = try await MediaImagePipeline.make(places: media).image(for: request)
      server.stop()
      let second = try await MediaImagePipeline.make(places: media).image(for: request)

      #expect(pixelWidth(first) == 4)
      #expect(pixelWidth(second) == 4)
      #expect(server.headers(of: "/image.png")["authorization"] == "Bearer t")
      #expect(MediaImagePipeline.make(places: media).configuration.dataCache == nil)
    }
  }
}
