import CoreGraphics
import Foundation
import MediaLayer
import Nuke
import QuartzCore
import Testing
@testable import MediaLayerUI

struct VisibleCall: Equatable {
  let url: String
  let visible: Bool
}

final class Latch: @unchecked Sendable {
  private let lock = NSLock()
  private var isOpen = false
  private var waiting: [CheckedContinuation<Void, Never>] = []

  func wait() async {
    await withCheckedContinuation { continuation in
      let open = lock.withLock { () -> Bool in
        if !isOpen { waiting.append(continuation) }
        return isOpen
      }
      if open { continuation.resume() }
    }
  }

  func open() {
    let released = lock.withLock { () -> [CheckedContinuation<Void, Never>] in
      isOpen = true
      defer { waiting = [] }
      return waiting
    }
    released.forEach { $0.resume() }
  }
}

extension FfiQueue {
  static func drain() async { await run { } }
}

final class RecordingPlaces: MediaPlaces, @unchecked Sendable {
  var imageOrigin: ImageOrigin = .network
  private let counters = ImageCounters()
  func imageStats() -> ImageCounters {
    if Thread.isMainThread { lock.withLock { mainStats += 1 } }
    noteMain("imageStats")
    return counters
  }
  func cacheKey(url: String, headers: [String: String]) -> String {
    lock.withLock { keyReads += 1 }
    noteMain("cacheKey")
    return url
  }
  private var mainCalls: [String] = []
  var mainThreadCalls: [String] { lock.withLock { mainCalls } }
  func resetMainThreadCalls() { lock.withLock { mainCalls = [] } }
  private func noteMain(_ call: String) { if Thread.isMainThread { lock.withLock { mainCalls.append(call) } } }
  private var keyReads = 0
  private var mainStats = 0
  var mainThreadImageStats: Int { lock.withLock { mainStats } }
  func resetMainThreadImageStats() { lock.withLock { mainStats = 0 } }
  var cacheKeyReads: Int { lock.withLock { keyReads } }
  private let lock = NSLock()
  private var visibility: [VisibleCall] = []
  var fetchLatch: Latch?

  func visibleCalls(of url: String) -> [Bool] { lock.withLock { visibility.filter { $0.url == url }.map(\.visible) } }
  private var preparation: TonePreparation?
  var needsTone = false
  var tonePreparation: TonePreparation? { lock.withLock { preparation } }

  private var calls: [String] = []
  var imageResult: Result<Data, Error> = .success(Data())
  private var fetches: [String] = []

  var fetchedURLs: [String] { lock.withLock { fetches } }

  var recorded: [String] { lock.withLock { calls } }

  private func record(_ call: String) { lock.withLock { calls.append(call) } }

  func visible(source: MediaItem, visible: Bool) {
    noteMain("visible")
    lock.withLock { visibility.append(VisibleCall(url: source.url, visible: visible)) }
    record("visible \(visible)")
  }
  func setPlaying(key: String, playing: Bool) { record("playing \(playing)") }
  func setPeeking(key: String, peeking: Bool) { record("peeking \(peeking)") }
  func setMuted(key: String, muted: Bool) { record("muted \(muted)") }
  func setMutedAll(muted: Bool) { record("mutedAll \(muted)") }
  func setRate(key: String, rate: Float) { record("rate \(rate)") }
  func seek(key: String, fraction: Double) { record("seek \(fraction)") }
  func itemState(key: String, state: MediaState) { noteMain("itemState"); record("state \(state)") }
  func fetchImage(url: String, headers: [String: String]) async throws -> ImageFetchResult {
    noteMain("fetchImage")
    lock.withLock { fetches.append(url) }
    if let latch = lock.withLock({ fetchLatch }) { await latch.wait() }
    return ImageFetchResult(bytes: try imageResult.get(), source: imageOrigin)
  }
  func placeholder(source: MediaItem, ready: Bool) async -> Placeholder { noteMain("placeholder"); return .none }
  func setTonePreparation(preparation: TonePreparation) { lock.withLock { self.preparation = preparation } }
  func tonePixelsNeeded(source: MediaItem) async -> Bool { needsTone }
  func playUrl(source: MediaItem) async throws -> PlaybackRoute {
    PlaybackRoute(url: source.url, headers: source.headers, generation: nil, hls: false, looping: false)
  }
  func stateStream(key: String) -> AsyncStream<MediaState> {
    noteMain("stateStream")
    let pair = AsyncStream<MediaState>.makeStream()
    lock.withLock { states.append(pair.continuation) }
    return pair.stream
  }
  private var states: [AsyncStream<MediaState>.Continuation] = []
  func emit(_ state: MediaState) { lock.withLock { states }.forEach { $0.yield(state) } }
  func attachView(key: String, token: UInt64) { record("attach \(key)") }
  func detachView(key: String) { record("detach \(key)") }
}

struct ViewModelTests {
  @MainActor
  @Test
  func inv_audio_01_onlyTheEndedStateOfTheCoreStopsTheAudio() {
    let places = RecordingPlaces()
    let audio = AudioPlayback(key: "note", playing: false, places: places)

    audio.toggle()
    audio.refresh(progress: 0.5)
    #expect(audio.progress == 0.5)
    audio.refresh(progress: 1)
    #expect(audio.isPlaying)
    audio.follow(.ended)

    #expect(places.recorded == ["playing true", "playing false"])
    #expect(!audio.isPlaying)
  }

  @MainActor
  @Test
  func theAudioRateIsKeptAndSentToTheCore() {
    let places = RecordingPlaces()
    let audio = AudioPlayback(key: "note", playing: false, rate: 1.5, places: places)
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
    #expect(HostLayers.of(places).layer(for: token) === layer)

    attachment = nil

    #expect(places.recorded == ["attach clip", "detach clip"])
    #expect(HostLayers.of(places).layer(for: token) == nil)
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
    ImageMemory.install(ImageCache(), on: places)
    places.imageResult = result

    let request = MediaImagePipeline.request(places: places, for: TestImageSource())!

    _ = await MediaImagePipeline.load(TestImageSource(), request: request, places: places, pipeline: MediaImagePipeline.make(places: places))

    await FfiQueue.drain()
    #expect(places.recorded.map { $0.hasPrefix("state failed") ? "state failed" : $0 } == expected)
  }

  @Test
  func theLastViewThatLeavesStopsThePlayBeforeItWithdrawsTheKey() {
    let places = RecordingPlaces()
    let claims = KeyClaims(places: places)
    let owner = UUID()
    let item = TestClip().item
    claims.visible(owner: owner, source: item, visible: true)
    claims.setPlaying(owner: owner, key: item.url, playing: true)

    claims.release(owner: owner, key: item.url)

    #expect(places.recorded == ["visible true", "playing true", "playing false", "visible false"])
  }

  @Test
  func aCancelledImageLoadDoesNotReportTheItemAsFailed() async {
    let places = RecordingPlaces()
    places.imageResult = .success(pngData(side: 8))
    let source = TestImageSource()
    let request = MediaImagePipeline.request(places: places, for: source)!
    let pipeline = MediaImagePipeline.make(places: places)

    let image = await Task {
      withUnsafeCurrentTask { $0?.cancel() }
      return await MediaImagePipeline.load(source, request: request, places: places, pipeline: pipeline)
    }.value

    #expect(image == nil)
    #expect(!places.recorded.contains { $0.hasPrefix("state failed") })
  }

  @Test
  func aCancelledLoadAnsweredFromTheMemoryCacheDeliversNothing() async {
    let places = RecordingPlaces()
    places.imageResult = .success(pngData(side: 8))
    let source = TestImageSource()
    let request = MediaImagePipeline.request(places: places, for: source)!
    let pipeline = MediaImagePipeline.make(places: places)
    let warm = await MediaImagePipeline.load(source, request: request, places: places, pipeline: pipeline)
    #expect(warm != nil)
    await FfiQueue.drain()
    let recorded = places.recorded.count

    let image = await Task {
      withUnsafeCurrentTask { $0?.cancel() }
      return await MediaImagePipeline.load(source, request: request, places: places, pipeline: pipeline)
    }.value

    #expect(image == nil)
    await FfiQueue.drain()
    #expect(places.recorded.count == recorded)
  }

  @Test
  func twoViewsOfOneKeyKeepItVisibleUntilTheLastOneLeaves() {
    let places = RecordingPlaces()
    let claims = KeyClaims(places: places)
    let first = UUID(), second = UUID()
    let item = TestClip().item

    claims.visible(owner: first, source: item, visible: true)
    claims.visible(owner: second, source: item, visible: true)
    claims.visible(owner: first, source: item, visible: false)
    #expect(places.recorded == ["visible true"])

    claims.visible(owner: second, source: item, visible: false)
    #expect(places.recorded == ["visible true", "visible false"])
  }

  @Test
  func aViewThatDoesNotPlayNeverStopsAnotherViewOfTheSameKey() {
    let places = RecordingPlaces()
    let claims = KeyClaims(places: places)
    let playing = UUID(), idle = UUID()

    claims.setPlaying(owner: playing, key: "k", playing: true)
    claims.setPlaying(owner: idle, key: "k", playing: false)
    #expect(places.recorded == ["playing true"])

    claims.setPlaying(owner: playing, key: "k", playing: false)
    #expect(places.recorded == ["playing true", "playing false"])
  }

  @Test
  func aPeekedKeyIsForwardedOnceAndWithdrawnWhenItsLastViewLeaves() {
    let places = RecordingPlaces()
    let claims = KeyClaims(places: places)
    let first = UUID(), second = UUID()

    claims.setPeeking(owner: first, key: "k", peeking: true)
    claims.setPeeking(owner: second, key: "k", peeking: true)
    claims.release(owner: first, key: "k")
    #expect(places.recorded == ["peeking true"])

    claims.release(owner: second, key: "k")
    #expect(places.recorded == ["peeking true", "peeking false"])
  }

  @Test
  func aKeyIsMutedOnlyWhileEveryViewOfItAsksForMute() {
    let places = RecordingPlaces()
    let claims = KeyClaims(places: places)
    let first = UUID(), second = UUID()

    claims.setMuted(owner: first, key: "k", muted: true)
    claims.setMuted(owner: second, key: "k", muted: false)
    claims.setMuted(owner: second, key: "k", muted: true)

    #expect(places.recorded == ["muted true", "muted false", "muted true"])
  }

  @Test
  func aViewThatLeavesDropsItsPlayAndMuteClaims() {
    let places = RecordingPlaces()
    let claims = KeyClaims(places: places)
    let owner = UUID()
    claims.setPlaying(owner: owner, key: "k", playing: true)

    claims.release(owner: owner, key: "k")

    #expect(places.recorded == ["playing true", "playing false"])
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
