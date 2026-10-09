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
  private var fetches: [String] = []

  var fetchedURLs: [String] { lock.withLock { fetches } }

  var recorded: [String] { lock.withLock { calls } }

  private func record(_ call: String) { lock.withLock { calls.append(call) } }

  func visible(source: MediaItem, visible: Bool) { record("visible \(visible)") }
  func setPlaying(key: String, playing: Bool) { record("playing \(playing)") }
  func setMuted(key: String, muted: Bool) { record("muted \(muted)") }
  func setMutedAll(muted: Bool) { record("mutedAll \(muted)") }
  func setRate(key: String, rate: Float) { record("rate \(rate)") }
  func seek(key: String, positionMs: UInt64) { record("seek \(positionMs)") }
  func itemState(key: String, state: MediaState) { record("state \(state)") }
  func fetchImage(url: String, headers: [String: String]) async throws -> Data {
    lock.withLock { fetches.append(url) }
    return try imageResult.get()
  }
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

  @MainActor
  @Test
  func theAudioRateIsKeptAndSentToTheCore() {
    let places = RecordingPlaces()
    let audio = AudioPlayback(key: "note", playing: false, rate: 1.5, places: places, slots: LoadedSlots())
    #expect(audio.rate == 1.5)

    audio.setRate(2)

    #expect((audio.rate, places.recorded) == (2, ["rate 2.0"]))
  }

  @Test
  func aVideoDeclaresItselfLoopingUnlessTheCallerAsksForOneShot() {
    let clip = TestClip()

    #expect(MediaVideo(clip).item.looping)
    #expect(!MediaVideo(clip, loop: false, onFirstFrame: {}, onEnd: {}).item.looping)
  }

  @Test
  func firstFrameFiresOncePerLoadAndEndFiresEveryTimeTheClipEnds() {
    var edges = VideoEdges()
    let steps: [(MediaState, firstFrame: Bool, ended: Bool)] = [
      (.loading, false, false), (.ready, false, false), (.rendered, true, false), (.rendered, false, false),
      (.ended, false, true), (.rendered, false, false), (.ended, false, true), (.loading, false, false), (.rendered, true, false)
    ]

    for (state, firstFrame, ended) in steps {
      let reached = edges.advance(to: state)
      #expect((reached.firstFrame, reached.ended) == (firstFrame, ended), "\(state)")
    }
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

  @Test
  func releasingAnOlderViewOfTheSameKeyDoesNotDetachTheNewerView() {
    let places = RecordingPlaces()
    var older: SurfaceAttachment? = SurfaceAttachment(places: places, key: "replaced", layer: CALayer())
    let newer = SurfaceAttachment(places: places, key: "replaced", layer: CALayer())

    older = nil

    #expect(places.recorded == ["attach replaced", "attach replaced"])
    withExtendedLifetime((older, newer)) {}
  }

  @Test(arguments: [
    (Result<Data, Error>.success(pngData(side: 8)), ["state loading", "state ready"]),
    (Result<Data, Error>.failure(URLError(.notConnectedToInternet)), ["state loading", "state failed"])
  ])
  func imageLoadReportsItsStateToTheCore(result: Result<Data, Error>, expected: [String]) async {
    let places = RecordingPlaces()
    places.imageResult = result

    let request = MediaImagePipeline.request(for: TestImageSource())!

    _ = await MediaImagePipeline.load(TestImageSource(), request: request, places: places, pipeline: MediaImagePipeline.make(places: places))

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

private struct TestClip: MediaSource {
  var url: String { "file:///bundle/intro.mp4" }
  var kind: Kind { .video }
  var headers: [String: String] { [:] }
}
