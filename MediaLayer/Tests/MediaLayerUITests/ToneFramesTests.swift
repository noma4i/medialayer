import AVFoundation
import CoreGraphics
import Foundation
import ImageIO
import MediaLayer
import Nuke
import Testing
import UniformTypeIdentifiers
@testable import MediaLayerUI

private final class OfflineFetcher: Fetcher, @unchecked Sendable {
  func fetch(request: FetchRequest) async throws -> FetchResponse {
    throw MediaError.Fetch(detail: "offline")
  }
}

private struct Poster: MediaSource {
  let url: String
  var kind: Kind { .image }
  var headers: [String: String] { [:] }
}

private func solid(width: Int, height: Int, red: UInt8, green: UInt8, blue: UInt8) throws -> CGImage {
  var bytes = [UInt8](repeating: 255, count: width * height * 4)
  for index in 0 ..< width * height {
    bytes[index * 4] = red
    bytes[index * 4 + 1] = green
    bytes[index * 4 + 2] = blue
  }
  let context = try #require(bytes.withUnsafeMutableBytes { buffer in
    CGContext(
      data: buffer.baseAddress, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    )?.makeImage()
  })
  return context
}

private func writePNG(_ image: CGImage) throws -> String {
  let file = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).png")
  let destination = try #require(CGImageDestinationCreateWithURL(file as CFURL, UTType.png.identifier as CFString, 1, nil))
  CGImageDestinationAddImage(destination, image, nil)
  #expect(CGImageDestinationFinalize(destination))
  return file.absoluteString
}

private func openLayer() throws -> MediaLayer {
  try MediaLayer.open(
    fetcher: OfflineFetcher(),
    settings: UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!),
    directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString),
    silent: true
  )
}

private func pixelsTone(_ media: MediaLayer, _ item: MediaItem) async -> Bool {
  (try? await media.tone(source: item, band: 0.15))?.source == .pixels
}

private final class ToneChanges: ToneListener, @unchecked Sendable {
  private let lock = NSLock()
  private var values: [ToneSource] = []
  var count: Int { lock.withLock { values.count } }
  func changed(source: ToneSource) { lock.withLock { values.append(source) } }
}

extension GlobalMedia {
  struct ToneFramesTests {
    private static var clip: String {
      Bundle.module.url(forResource: "silent", withExtension: "mp4", subdirectory: "Fixtures")!.absoluteString
    }

    @Test
    func inv_tone_13_late_subscription_prepares_pixels_without_a_second_image_load() async throws {
      let media = try openLayer()
      let source = Poster(url: try writePNG(solid(width: 1080, height: 1920, red: 20, green: 40, blue: 60)))
      let context = MediaContext(places: media)
      let request = try #require(MediaImagePipeline.request(places: media, for: source))
      #expect(await MediaImagePipeline.load(source, request: request, places: media, pipeline: context.pipeline) != nil)
      #expect(await media.tonePixelsNeeded(source: source.item) == false)
      let changes = ToneChanges()
      let subscription = media.subscribeTone(source: source.item, listener: changes)
      await eventually("late subscriber receives pixels") { changes.count == 1 }
      #expect(await pixelsTone(media, source.item))
      #expect(await media.tonePixelsNeeded(source: source.item) == false)
      withExtendedLifetime(subscription) {}
    }

    @Test
    func inv_tone_13_no_demand_reads_no_bytes_and_thumbnail_decodes_at_most_64() async throws {
      let places = RecordingPlaces()
      let item = MediaItem(kind: .image, url: "https://o.test/large.png", headers: [:], cover: nil, blurHash: nil, strategy: .window)
      let file = try writePNG(solid(width: 1080, height: 1920, red: 20, green: 40, blue: 60))
      let data = try Data(contentsOf: URL(string: file)!)
      places.imageResult = .success(data)
      let preparation = ToneFrames(places: places)
      #expect(await preparation.prepare(source: item) == nil)
      #expect(places.fetchedURLs.isEmpty)
      let thumbnail = try #require(ToneFrames.thumbnail(data))
      #expect(thumbnail.width <= 64 && thumbnail.height <= 64)
      #expect(thumbnail.bytesPerRow * thumbnail.height <= 16384)
      places.needsTone = true
      let pixels = try #require(await preparation.prepare(source: item))
      #expect(places.fetchedURLs == [item.url])
      #expect(pixels.rgba.count <= 16384)
      print("tone_preparations=1 decoded_bytes=\(thumbnail.bytesPerRow * thumbnail.height) submitted_bytes=\(pixels.rgba.count)")
    }

    @Test(arguments: [(200, 100, 64, 32), (100, 300, 21, 64), (30, 20, 30, 20)])
    func inv_tone_12_a_decoded_image_is_shrunk_to_64_on_the_long_side_keeping_its_colours(
      width: Int, height: Int, expectedWidth: Int, expectedHeight: Int
    ) throws {
      let shrunk = try #require(ToneFrames.rgba(from: solid(width: width, height: height, red: 200, green: 20, blue: 40)))

      #expect((shrunk.width, shrunk.height) == (expectedWidth, expectedHeight))
      #expect(shrunk.rgba.count == expectedWidth * expectedHeight * 4)
      #expect(abs(Int(shrunk.rgba[0]) - 200) <= 2 && abs(Int(shrunk.rgba[1]) - 20) <= 2 && abs(Int(shrunk.rgba[2]) - 40) <= 2)
    }

    @MainActor
    @Test
    func inv_tone_12_the_pixels_reach_the_core_off_the_main_thread() async throws {
      let places = RecordingPlaces()
      let item = MediaItem(kind: .image, url: "https://o.test/a.png", headers: [:], cover: nil, blurHash: nil, strategy: .window)

      let file = try writePNG(solid(width: 300, height: 200, red: 1, green: 2, blue: 3))
      places.imageResult = .success(try Data(contentsOf: URL(string: file)!))
      places.needsTone = true
      let preparation = ToneFrames(places: places)
      let pixels = try #require(await preparation.prepare(source: item))
      #expect(pixels.width == 64 && pixels.height <= 43)
      #expect(pixels.rgba.count == Int(pixels.width * pixels.height * 4))
    }

    @Test
    func inv_tone_12_a_failed_decode_submits_nothing() async throws {
      let places = RecordingPlaces()
      let item = MediaItem(kind: .video, url: "file:///nonexistent/clip.mp4", headers: [:], cover: nil, blurHash: nil, strategy: .window)

      places.needsTone = true
      let preparation = ToneFrames(places: places)
      #expect(await preparation.prepare(source: item) == nil)
    }

    @Test
    func inv_tone_12_a_loaded_image_makes_the_core_answer_with_pixels() async throws {
      let media = try openLayer()
      let source = Poster(url: try writePNG(solid(width: 120, height: 80, red: 250, green: 10, blue: 10)))
      let context = MediaContext(places: media)
      let request = try #require(MediaImagePipeline.request(places: media, for: source, points: CGSize(width: 40, height: 40), scale: 2))

      let container = await MediaImagePipeline.load(source, request: request, places: media, pipeline: context.pipeline)

      #expect(container != nil)
      await eventually("tone answers with pixels") { await pixelsTone(media, source.item) }
    }

    @Test
    func inv_tone_12_a_video_cover_gives_the_video_its_tone() async throws {
      let media = try openLayer()
      let cover = try writePNG(solid(width: 90, height: 160, red: 10, green: 10, blue: 240))
      let video = MediaItem(kind: .video, url: "https://o.test/v.mp4", headers: [:], cover: cover, blurHash: nil, strategy: .window)

      let image = await PlaceholderView.loadCover(cover, for: video, context: MediaContext(places: media), points: CGSize(width: 45, height: 80), scale: 2)

      #expect(image != nil)
      await eventually("the video tone answers with pixels") { await pixelsTone(media, video) }
    }

    @Test
    func inv_tone_12_a_video_without_a_cover_gives_its_first_frame_and_a_covered_one_does_not() async throws {
      let media = try openLayer()
      let bare = MediaItem(kind: .video, url: Self.clip, headers: [:], cover: nil, blurHash: nil, looping: true, strategy: .window)

      media.visible(source: bare, visible: true)
      media.setPlaying(key: Self.clip, playing: true)
      await eventually("the clip is loaded") { LoadedSlots.of(media).slot(for: Self.clip)?.loadedItem != nil }

      #expect(await media.tonePixelsNeeded(source: bare) == false)
      #expect(await pixelsTone(media, bare) == false)

      await eventually("the first frame gives pixels") { await pixelsTone(media, bare) }
    }
  }
}
