import CoreGraphics
import Foundation
import MediaLayer
import Nuke
import QuartzCore
import Testing
@testable import MediaLayerUI

final class RecordingPlaces: MediaPlaces, @unchecked Sendable {
  private let lock = NSLock()
  private var calls: [String] = []
  var imageResult: Result<Data, Error> = .success(Data())

  var recorded: [String] { lock.withLock { calls } }

  private func record(_ call: String) { lock.withLock { calls.append(call) } }

  func visible(source: MediaItem, visible: Bool) { record("visible \(visible)") }
  func setPlaying(key: String, playing: Bool) { record("playing \(playing)") }
  func setMuted(key: String, muted: Bool) { record("muted \(muted)") }
  func setMutedAll(muted: Bool) { record("mutedAll \(muted)") }
  func seek(key: String, positionMs: UInt64) { record("seek \(positionMs)") }
  func itemState(key: String, state: MediaState) { record("state \(state)") }
  func fetchImage(url: String, headers: [String: String]) async throws -> Data { try imageResult.get() }
  func placeholder(source: MediaItem, ready: Bool) async -> Placeholder { .none }
  func stateStream(key: String) -> AsyncStream<MediaState> { AsyncStream { $0.finish() } }
  func attachView(key: String, token: UInt64) { record("attach \(key)") }
  func detachView(key: String) { record("detach \(key)") }
}

struct ViewModelTests {
  @MainActor
  @Test
  func toggleAndReachingTheEndDriveSetPlaying() {
    let places = RecordingPlaces()
    let audio = AudioPlayback(key: "note", playing: false, places: places, slots: LoadedSlots())

    audio.toggle()
    audio.refresh(from: SlotSample(positionMs: 500, durationMs: 1000, isLooping: false, viewAttached: false, frameRendered: false))
    #expect(audio.progress == 0.5)
    audio.refresh(from: SlotSample(positionMs: 1000, durationMs: 1000, isLooping: false, viewAttached: false, frameRendered: false))

    #expect(places.recorded == ["playing true", "playing false"])
    #expect(!audio.isPlaying)
  }

  @Test
  func viewAttachmentRegistersItsLayerWithTheCoreAndDetachesOnRelease() {
    let places = RecordingPlaces()
    let layer = CALayer()
    var attachment: SurfaceAttachment? = SurfaceAttachment(places: places, key: "clip", layer: layer)
    let token = attachment!.token
    #expect(HostLayers.shared.layer(for: token) === layer)

    attachment = nil

    #expect(places.recorded == ["attach clip", "detach clip"])
    #expect(HostLayers.shared.layer(for: token) == nil)
  }

  @Test(arguments: [
    (Result<Data, Error>.success(pngData(side: 8)), ["state loading", "state ready"]),
    (Result<Data, Error>.failure(URLError(.notConnectedToInternet)), ["state loading", "state failed"])
  ])
  func imageLoadReportsItsStateToTheCore(result: Result<Data, Error>, expected: [String]) async {
    let places = RecordingPlaces()
    places.imageResult = result

    _ = await MediaImagePipeline.load(TestImageSource(), places: places, pipeline: MediaImagePipeline.make(places: places))

    #expect(places.recorded.map { $0.hasPrefix("state failed") ? "state failed" : $0 } == expected)
  }

  @Test
  func releaserDropsTheNukeMemoryCache() {
    let request = ImageRequest(url: URL(string: "https://o.test/cached.png"))
    ImageCache.shared[ImageCacheKey(request: request)] = ImageContainer(image: PlatformImage())
    #expect(ImageCache.shared[ImageCacheKey(request: request)] != nil)

    ImageMemoryReleaser().dropMemoryCaches()

    #expect(ImageCache.shared[ImageCacheKey(request: request)] == nil)
  }

  @Test(arguments: [("LEHV6nWB2yk8pyo0adR*.7kCMdnj", true), ("x", false)])
  func blurHashDecodesThroughTheCoreOrIsRejected(hash: String, valid: Bool) {
    let pixels = PlaceholderPixels.blurHash(hash)

    #expect((pixels?.cgImage != nil) == valid)
  }

  @Test(arguments: [(2, 1, 8, true), (2, 1, 7, false), (0, 1, 0, false)])
  func rgbaPixelsBecomeAnImageOnlyWhenTheBufferMatches(width: Int, height: Int, bytes: Int, valid: Bool) {
    let pixels = PlaceholderPixels(width: width, height: height, rgba: Data(count: bytes))

    #expect((pixels.cgImage != nil) == valid)
  }
}

private struct TestImageSource: MediaSource {
  var url: String { "https://o.test/a.png" }
  var kind: Kind { .image }
  var headers: [String: String] { [:] }
}
