import CoreGraphics
import Foundation
import MediaLayer
import Nuke
import Testing
@testable import MediaLayerUI

private struct LocalImage: MediaSource {
  let url: String
  var kind: Kind { .image }
  var headers: [String: String] { [:] }
}

private final class NoOrigin: Fetcher, @unchecked Sendable {
  func fetch(request: FetchRequest) async throws -> FetchResponse { throw MediaError.Fetch(detail: "offline") }
}

private func write(_ data: Data) throws -> URL {
  let url = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).png")
  try data.write(to: url)
  return url
}

extension GlobalMedia {
  struct LocalImageThroughCoreTests {
    @Test
    func inv_image_49_a_file_image_reaches_the_decoder_through_fetch_image() async throws {
      let places = RecordingPlaces()
      let missing = URL(fileURLWithPath: "/nonexistent/\(UUID().uuidString).png")
      places.imageResult = .success(pngData(side: 8))
      ImageMemory.install(ImageCache(), on: places)
      let pipeline = MediaImagePipeline.make(places: places)
      let request = try #require(MediaImagePipeline.request(places: places, for: LocalImage(url: missing.absoluteString), points: CGSize(width: 8, height: 8), contentMode: .fill))

      let image = try await pipeline.image(for: request)

      #expect(image.size.width > 0)
      #expect(places.fetchedURLs == [missing.absoluteString])
    }

    @Test
    func inv_image_49_a_file_above_the_response_limit_fails_and_a_small_one_loads() async throws {
      let media = try await MediaLayer.open(fetcher: NoOrigin(), settings: UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!), directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString))
      let small = try write(pngData(side: 8))
      let huge = try write(Data(count: 16 * 1024 * 1024 + 1))
      defer { try? FileManager.default.removeItem(at: small); try? FileManager.default.removeItem(at: huge) }
      ImageMemory.install(ImageCache(), on: media)
      let pipeline = MediaImagePipeline.make(places: media)
      func load(_ file: URL) async throws -> PlatformImage {
        let request = try #require(MediaImagePipeline.request(places: media, for: LocalImage(url: file.absoluteString), points: CGSize(width: 8, height: 8), contentMode: .fill))
        return try await pipeline.image(for: request)
      }

      #expect(try await load(small).size.width > 0)
      do {
        _ = try await load(huge)
        Issue.record("a file above the response limit loaded")
      } catch ImagePipeline.Error.dataLoadingFailed(let underlying) {
        #expect(underlying is MediaError)
      } catch {
        Issue.record("the failure did not come from the core: \(error)")
      }
      withExtendedLifetime(media) {}
    }

    @Test
    func inv_api_47_a_lone_local_image_place_is_declared_ready_without_a_window() async throws {
      let media = try await openCore()
      let url = "file:///bundle/photo.png"
      let stream = media.stateStream(key: url)

      let place = try media.mountPlace(item: MediaItem(kind: .image, url: url, headers: [:], cover: nil, blurHash: nil, strategy: .window))
      try place.update(input: PlaceInput(visible: true, playing: false, group: nil, peeking: false, muted: nil, rate: nil, pinned: false, viewToken: nil))

      #expect(await first(of: stream) { $0.isReady } != nil)
      place.release()
      withExtendedLifetime(media) {}
    }

    @Test
    func inv_api_47_a_lone_local_image_place_reports_the_failure_of_its_view_without_a_window() async throws {
      let media = try await openCore()
      let url = "file:///bundle/broken.png"
      let declared = media.stateStream(key: url)

      let place = try media.mountPlace(item: MediaItem(kind: .image, url: url, headers: [:], cover: nil, blurHash: nil, strategy: .window))
      try place.update(input: PlaceInput(visible: true, playing: false, group: nil, peeking: false, muted: nil, rate: nil, pinned: false, viewToken: nil))
      #expect(await first(of: declared) { $0.isReady } != nil)
      let failures = media.stateStream(key: url)
      try place.itemState(state: .failed(detail: "undecodable"))

      #expect(await first(of: failures) { if case .failed = $0 { return true } else { return false } } != nil)
      place.release()
      withExtendedLifetime(media) {}
    }

    private func openCore() async throws -> MediaLayer {
      try await MediaLayer.open(fetcher: NoOrigin(), settings: UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!), directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString))
    }

    private func first(of stream: AsyncStream<MediaState>, where match: @escaping @Sendable (MediaState) -> Bool) async -> MediaState? {
      await withTaskGroup(of: MediaState?.self) { group in
        group.addTask { for await state in stream where match(state) { return state }; return nil }
        group.addTask { try? await Task.sleep(for: .seconds(5)); return nil }
        let found = await group.next() ?? nil
        group.cancelAll()
        return found
      }
    }
  }
}
