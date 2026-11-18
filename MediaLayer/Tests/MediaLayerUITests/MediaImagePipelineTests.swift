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

private final class ResizeProbe: ImageProcessing, @unchecked Sendable {
  let processor: any ImageProcessing
  private let lock = NSLock()
  private var changed = 0
  private var called = 0
  var copies: Int { lock.withLock { changed } }
  var calls: Int { lock.withLock { called } }
  var identifier: String { processor.identifier }

  init(_ processor: any ImageProcessing) { self.processor = processor }

  func process(_ image: PlatformImage) -> PlatformImage? {
    let result = processor.process(image)
    lock.withLock {
      called += 1
      if result?.cgImage !== image.cgImage { changed += 1 }
    }
    return result
  }
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
    @Test(arguments: [ContentMode.fill, ContentMode.fit])
    func inv_image_36_thumbnail_does_not_resample_twice(mode: ContentMode) async throws {
      let places = RecordingPlaces()
      places.imageResult = .success(pngData(side: 1080, height: 1920))
      ImageMemory.install(ImageCache(), on: places)
      let pipeline = MediaImagePipeline.make(places: places)
      var request = try #require(MediaImagePipeline.request(places: places, for: TestImage(url: "https://o.test/resize.png"), points: CGSize(width: 352, height: 352), contentMode: mode))
      let probe = ResizeProbe(try #require(request.processors.first))
      request.processors = [probe]
      _ = try await pipeline.image(for: request)
      #expect(probe.calls == 1)
      #expect(probe.copies == 0)
    }

    @Test
    func inv_placeholder_05_a_video_cover_decodes_no_more_than_its_placement() async throws {
      let places = RecordingPlaces()
      places.imageResult = .success(pngData(side: 2000))
      ImageMemory.install(ImageCache(), on: places)
      let video = MediaItem(kind: .video, url: "https://o.test/v.mp4", headers: [:], cover: "https://o.test/cover.png", blurHash: nil, strategy: .window)

      let image = await PlaceholderView.loadCover("https://o.test/cover.png", for: video, context: MediaContext(places: places), points: CGSize(width: 50, height: 50), scale: 2)
      await eventually("the cover decode is counted") { let applied = places.imageStats().snapshot(); return applied.requests == 1 && applied.activeRequests == 0 }
      let stats = places.imageStats().snapshot()
      #expect(image != nil)
      #expect(stats.decodes == 1)
      #expect(stats.placementPixels == 100 * 100)
      #expect(stats.decodedPixels <= 104 * 104)
    }

    @Test
    func inv_native_21_contexts_share_only_their_open_object_resources() {
      let places = RecordingPlaces()
      let first = MediaContext(places: places)
      let second = MediaContext(places: places)
      let other = MediaContext(places: RecordingPlaces())
      #expect(first.pipeline === second.pipeline)
      #expect(first.slots === second.slots)
      #expect(first.slots !== other.slots)
    }

    @Test
    func inv_image_30_memory_lookup_does_not_count_a_request() async throws {
      let places = RecordingPlaces()
      places.imageResult = .success(pngData(side: 16))
      ImageMemory.install(ImageCache(), on: places)
      let pipeline = MediaImagePipeline.make(places: places)
      let request = try #require(MediaImagePipeline.request(places: places, for: TestImage(url: "https://o.test/lookup.png"), points: CGSize(width: 16, height: 16)))
      _ = try await pipeline.image(for: request)
      await eventually("the request is counted") { let applied = places.imageStats().snapshot(); return applied.requests == 1 && applied.activeRequests == 0 }
      let before = places.imageStats().snapshot()
      for _ in 0 ..< 3 {
        #expect(MediaImagePipeline.cachedContainer(for: request, in: pipeline) != nil)
      }
      let after = places.imageStats().snapshot()
      #expect(after.requests == before.requests)
      #expect(after.placementPixels == before.placementPixels)
    }

    @Test
    func inv_image_13_counts_decoding_disk_and_memory_separately() async throws {
      let places = RecordingPlaces()
      places.imageResult = .success(pngData(side: 16))
      places.imageOrigin = .cache
      ImageMemory.install(ImageCache(), on: places)
      let pipeline = MediaImagePipeline.make(places: places)
      let request = try #require(MediaImagePipeline.request(places: places, for: TestImage(url: "https://o.test/count.png"), points: CGSize(width: 16, height: 16)))
      _ = try await pipeline.image(for: request)
      await eventually("the request is counted") { let applied = places.imageStats().snapshot(); return applied.requests == 1 && applied.activeRequests == 0 }
      #expect(places.imageStats().snapshot().placementPixels == 256)
      #expect(MediaImagePipeline.cachedContainer(for: request, in: pipeline) != nil)
      _ = try await pipeline.image(for: request)
      await eventually("the records are applied") { let applied = places.imageStats().snapshot(); return applied.memoryHits == 1 && applied.activeRequests == 0 }
      let stats = places.imageStats().snapshot()
      #expect(stats.requests == 2)
      #expect(stats.decodes == 1)
      #expect(stats.diskHits == 1)
      #expect(stats.memoryHits == 1)
      #expect(stats.decodedPixels == 256)
      #expect(stats.bitmapBytes >= 1024)
      #expect(stats.activeRequests == 0)
      #expect(stats.placementPixels == 512)
    }

    @Test(arguments: [(ContentMode.fill, 352, 626), (ContentMode.fit, 198, 352)])
    func inv_image_11_placement_caps_decoded_pixels(mode: ContentMode, width: Int, height: Int) async throws {
      let places = RecordingPlaces()
      places.imageResult = .success(pngData(side: 1080, height: 1920))
      ImageMemory.install(ImageCache(), on: places)
      let pipeline = MediaImagePipeline.make(places: places)
      let source = TestImage(url: "https://o.test/portrait.png", size: MediaSize(width: 1080, height: 1920))
      let request = try #require(MediaImagePipeline.request(places: places, for: source, points: CGSize(width: 352, height: 352), contentMode: mode))
      let image = try await pipeline.image(for: request)
      #expect(image.cgImage?.width == width)
      #expect(image.cgImage?.height == height)
      await eventually("the request is counted") { let applied = places.imageStats().snapshot(); return applied.requests == 1 && applied.activeRequests == 0 }
      let stats = places.imageStats().snapshot()
      #expect(stats.decodes == 1)
      #expect(stats.decodedPixels <= UInt64(width * height))
      #expect(stats.placementPixels == 352 * 352)
    }

    @Test
    func inv_image_11_the_counters_of_a_decode_arrive_with_the_recorder_drain() async throws {
      let places = RecordingPlaces()
      places.imageResult = .success(pngData(side: 1080, height: 1920))
      ImageMemory.install(ImageCache(), on: places)
      let pipeline = MediaImagePipeline.make(places: places)
      let source = TestImage(url: "https://o.test/portrait.png", size: MediaSize(width: 1080, height: 1920))
      let request = try #require(MediaImagePipeline.request(places: places, for: source, points: CGSize(width: 352, height: 352), contentMode: .fill))
      let held = HeldQueue()
      defer { held.release() }

      _ = try await pipeline.image(for: request)
      #expect(places.imageStats().snapshot().decodes == 0, "the drain applied while the queue was held")
      held.release()

      await eventually("the request is counted") { let applied = places.imageStats().snapshot(); return applied.requests == 1 && applied.activeRequests == 0 }
      let stats = places.imageStats().snapshot()
      #expect(stats.decodes == 1)
      #expect(stats.placementPixels == 352 * 352)
    }

    @Test
    func inv_memory_04_a_system_warning_clears_decoded_images() async throws {
      let media = try await MediaLayer.open(
        fetcher: URLSessionFetcher(),
        settings: UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!),
        directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
      )
      _ = try await media.stats()
      let pipeline = MediaImagePipeline.make(places: media)
      let request = try #require(MediaImagePipeline.request(places: media, for: TestImage(url: "https://o.test/warning.png")))
      pipeline.cache[request] = ImageContainer(image: try ImageDecoders.Default().decode(pngData(side: 4)).image)
      #expect(pipeline.cache[request] != nil)

      NotificationCenter.default.post(name: SystemSignalNames.system.memoryWarning, object: nil)

      await eventually("the system warning cleared decoded images") { pipeline.cache[request] == nil }
      #expect(pipeline.cache[request] == nil)
    }

    @Test(arguments: [(MediaSize?.some(MediaSize(width: 8, height: 8)), 1), (MediaSize?.none, 0)])
    func requestResizesToTheDeclaredSizeAndForwardsHeaders(size: MediaSize?, processors: Int) throws {
      let request = try #require(MediaImagePipeline.request(places: RecordingPlaces(), for: TestImage(url: "https://o.test/a.png", headers: ["X-Token": "t"], size: size)))

      #expect(request.processors.count == processors)
      #expect(request.urlRequest?.value(forHTTPHeaderField: "X-Token") == "t")
    }

    @Test(arguments: [
      (CGSize(width: 50, height: 50), CGFloat(3), MediaSize?.none, "(152.0, 152.0)"),
      (CGSize(width: 100.4, height: 20), CGFloat(2), MediaSize?.none, "(208.0, 40.0)"),
      (CGSize(width: 50, height: 50), CGFloat(3), MediaSize?.some(MediaSize(width: 16, height: 16)), "(16.0, 16.0)"),
      (CGSize(width: 0, height: 0), CGFloat(3), MediaSize?.some(MediaSize(width: 16, height: 16)), "(16.0, 16.0)")
    ])
    func inv_image_01_the_request_decodes_at_the_view_size_in_pixels_rounded_up_to_8_without_upscaling_the_source(
      points: CGSize, scale: CGFloat, declared: MediaSize?, expected: String
    ) throws {
      let request = MediaImagePipeline.request(places: RecordingPlaces(), for: TestImage(url: "https://o.test/a.png", size: declared), points: points, scale: scale)

      #expect(resizeIdentifier(request).contains("s=\(expected)"))
    }

    @Test
    func inv_image_01_a_view_without_size_or_a_declared_size_has_nothing_to_decode_at() throws {
      let request = try #require(MediaImagePipeline.request(places: RecordingPlaces(), for: TestImage(url: "https://o.test/a.png"), points: .zero, scale: 3))

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
        _ = try await pipeline.image(for: try #require(MediaImagePipeline.request(places: places, for: source, points: CGSize(width: side, height: side), scale: 2)))
      }

      #expect(places.fetchedURLs == ["https://o.test/a.png", "https://o.test/a.png"])
    }

    @Test
    func inv_image_04_the_content_mode_picks_the_resize_mode_and_keeps_the_decode_size() throws {
      let source = TestImage(url: "https://o.test/a.png")
      let fill = resizeIdentifier(MediaImagePipeline.request(places: RecordingPlaces(), for: source, points: CGSize(width: 50, height: 50), scale: 3, contentMode: .fill))
      let fit = resizeIdentifier(MediaImagePipeline.request(places: RecordingPlaces(), for: source, points: CGSize(width: 50, height: 50), scale: 3, contentMode: .fit))

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
      let held = Latch()
      places.fetchLatch = held
      let cold = shownFirstFrame()
      held.open()
      func fillMemory() async throws {
        let request = try #require(MediaImagePipeline.request(places: places, for: source, points: CGSize(width: 50, height: 50), scale: 2))
        _ = try await context.pipeline.image(for: request)
      }
      try await fillMemory()
      for index in 0 ..< 70 {
        _ = MediaImagePipeline.request(places: places, for: TestImage(url: "https://o.test/metadata-\(index).png"), points: CGSize(width: 50, height: 50), scale: 2)
      }
      places.resetMainThreadCalls()
      let reads = places.cacheKeyReads
      let warm = shownFirstFrame()
      #expect(places.mainThreadCalls.isEmpty)
      #expect(places.cacheKeyReads == reads)

      let request = try #require(MediaImagePipeline.request(places: places, for: source, points: CGSize(width: 50, height: 50), scale: 2))
      let larger = try #require(MediaImagePipeline.request(places: places, for: source, points: CGSize(width: 80, height: 80), scale: 2))
      #expect(MediaImagePipeline.cachedImage(for: request, in: context.pipeline) != nil)
      #expect(MediaImagePipeline.cachedImage(for: larger, in: context.pipeline) == nil)
      #expect(cold?.last == 0)
      #expect(warm?.first.map { $0 > 200 } == true && warm?.last == 255)
    }

    @Test
    func inv_memory_10_the_decoder_memory_is_limited_by_the_open_option_and_a_non_positive_limit_is_refused() async throws {
      let settings = UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!)
      let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)

      let limited = try await MediaLayer.open(fetcher: URLSessionFetcher(), settings: settings, directory: directory, imageMemoryBytes: 5_000_000)
      let defaulted = try await MediaLayer.open(fetcher: URLSessionFetcher(), settings: settings, directory: directory.appendingPathComponent("b"))

      #expect(ImageMemory.cache(for: limited).costLimit + ImageMemory.frames(for: limited).byteLimit == 5_000_000)
      #expect(ImageMemory.cache(for: defaulted).costLimit == 128 * 1024 * 1024)
      await #expect(throws: MediaError.self) {
        try await MediaLayer.open(fetcher: URLSessionFetcher(), settings: settings, directory: directory.appendingPathComponent("c"), imageMemoryBytes: 0)
      }
    }

    @Test
    func inv_image_86_animated_frames_and_still_images_share_the_one_image_memory_budget() async throws {
      let settings = UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!)
      let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
      let tiny = try await MediaLayer.open(fetcher: URLSessionFetcher(), settings: settings, directory: directory, imageMemoryBytes: 1)
      #expect(ImageMemory.cache(for: tiny).costLimit + ImageMemory.frames(for: tiny).byteLimit == 1)
      let data = NSMutableData()
      let destination = CGImageDestinationCreateWithData(data, UTType.gif.identifier as CFString, 2, nil)!
      for _ in 0 ..< 2 {
        let context = CGContext(data: nil, width: 8, height: 8, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        CGImageDestinationAddImage(destination, context.makeImage()!, [kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFDelayTime: 0.1]] as CFDictionary)
      }
      CGImageDestinationFinalize(destination)
      let cache = ImageMemory.frames(for: tiny)
      _ = await cache.load(identity: "a", data: data as Data, maxPixel: 8, admit: { _ in {} })
      #expect(cache.bytes == 0 && cache.count == 0)
    }

    @Test
    func inv_image_03_clearing_the_cache_drops_the_decoder_memory() async throws {
      let media = try await MediaLayer.open(
        fetcher: URLSessionFetcher(),
        settings: UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!),
        directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
      )
      let pipeline = MediaImagePipeline.make(places: media)
      let request = try #require(MediaImagePipeline.request(places: media, for: TestImage(url: "https://o.test/a.png"), points: CGSize(width: 50, height: 50), scale: 2))
      pipeline.cache[request] = ImageContainer(image: try ImageDecoders.Default().decode(pngData(side: 4)).image)
      #expect(pipeline.cache[request] != nil)

      try! await media.clear(kind: .video)
      #expect(pipeline.cache[request] != nil)
      try! await media.clear(kind: nil)

      #expect(pipeline.cache[request] == nil)
    }

    @Test
    func imageLoadsThroughTheCoreAndASecondPipelineReadsTheCoreDiskCacheWithTheOriginDown() async throws {
      let server = try LocalHTTPServer()
      try await server.start()
      server.serve(pngData(side: 16), at: "/image.png")
      let media = try await MediaLayer.open(
        fetcher: URLSessionFetcher(),
        settings: UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!),
        directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
      )
      let source = TestImage(url: server.url("/image.png"), headers: ["Authorization": "Bearer t"], size: MediaSize(width: 4, height: 4))
      let request = try #require(MediaImagePipeline.request(places: media, for: source))

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
