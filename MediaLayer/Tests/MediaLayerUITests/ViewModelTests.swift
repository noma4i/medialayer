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
  var latchedURL: String?
  private var bytesByURL: [String: Data] = [:]
  func answer(_ url: String, with bytes: Data) { lock.withLock { bytesByURL[url] = bytes } }

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

  var placeFactory: ((String) -> any PlaceHandle)?
  var mountError: MediaError?
  var mutedAllError: MediaError?
  func mountPlace(item: MediaItem) throws -> any PlaceHandle {
    noteMain("mountPlace")
    record("mount")
    if let mountError { throw mountError }
    return placeFactory?(item.url) ?? RecordingPlace(places: self, url: item.url)
  }
  func mountVideoWindow() throws -> any WindowHandle { opened(RecordingWindow()) }
  func mountImageWindow(name: String, rowsPerPage: Int, reversed: Bool) throws -> any WindowHandle { opened(RecordingWindow()) }
  private var windows: [RecordingWindow] = []
  var openedWindows: [RecordingWindow] { lock.withLock { windows } }
  private func opened(_ window: RecordingWindow) -> RecordingWindow {
    lock.withLock { windows.append(window) }
    return window
  }
  func seek(key: String, fraction: Double) throws { record("seek \(key) \(fraction)") }
  func setMutedAll(muted: Bool) throws {
    if let mutedAllError { throw mutedAllError }
    record("mutedAll \(muted)")
  }
  fileprivate func note(visible: Bool, url: String) {
    lock.withLock { visibility.append(VisibleCall(url: url, visible: visible)) }
    record("visible \(visible)")
  }
  fileprivate func log(_ call: String) { record(call) }
  func fetchImage(url: String, headers: [String: String]) async throws -> ImageFetchResult {
    noteMain("fetchImage")
    lock.withLock { fetches.append(url) }
    if let latch = lock.withLock({ latchedURL == nil || latchedURL == url ? fetchLatch : nil }) { await latch.wait() }
    if let bytes = lock.withLock({ bytesByURL[url] }) { return ImageFetchResult(bytes: bytes, source: imageOrigin) }
    return ImageFetchResult(bytes: try imageResult.get(), source: imageOrigin)
  }
  func placeholder(source: MediaItem, ready: Bool) async -> Placeholder { noteMain("placeholder"); return .none }
  func setTonePreparation(preparation: TonePreparation) throws { lock.withLock { self.preparation = preparation } }
  func tonePixelsNeeded(source: MediaItem) async -> Bool { needsTone }
  func playUrl(source: MediaItem) async throws -> PlaybackRoute {
    PlaybackRoute(url: source.url, headers: source.headers, generation: nil, hls: false, looping: false)
  }
  struct ToneRequest: @unchecked Sendable {
    let source: MediaItem
    let band: Float
    let continuation: AsyncStream<ToneUpdate>.Continuation
  }
  private var toneAsked: [ToneRequest] = []
  private var toneEnded = 0
  var toneRequests: [ToneRequest] { lock.withLock { toneAsked } }
  var toneTerminations: Int { lock.withLock { toneEnded } }
  func toneStream(source: MediaItem, band: Float) -> AsyncStream<ToneUpdate> {
    let pair = AsyncStream<ToneUpdate>.makeStream()
    pair.continuation.onTermination = { [weak self] _ in self?.lock.withLock { self?.toneEnded += 1 } }
    lock.withLock { toneAsked.append(ToneRequest(source: source, band: band, continuation: pair.continuation)) }
    return pair.stream
  }
  func stateStream(key: String) -> AsyncStream<MediaState> {
    noteMain("stateStream")
    let pair = AsyncStream<MediaState>.makeStream()
    lock.withLock { states.append(pair.continuation) }
    return pair.stream
  }
  private var states: [AsyncStream<MediaState>.Continuation] = []
  func emit(_ state: MediaState) { lock.withLock { states }.forEach { $0.yield(state) } }
}

private final class RecordingPlace: PlaceHandle, @unchecked Sendable {
  private let places: RecordingPlaces
  private let url: String
  private let lock = NSLock()
  private var last = PlaceInput.make()
  private var released = false

  init(places: RecordingPlaces, url: String) {
    self.places = places
    self.url = url
  }

  func update(input: PlaceInput) throws {
    let before = lock.withLock { () -> PlaceInput in defer { last = input }; return last }
    if input.visible != before.visible { places.note(visible: input.visible, url: url) }
    if input.playing != before.playing || (input.playing && input.group != before.group) {
      places.log("playing \(input.playing)" + (input.group.map { " groups=\($0)" } ?? ""))
    }
    if input.peeking != before.peeking { places.log("peeking \(input.peeking)") }
    if let muted = input.muted, muted != before.muted { places.log("muted \(muted)") }
    if let rate = input.rate, rate != before.rate { places.log("rate \(rate)") }
    if input.viewToken != before.viewToken { places.log(input.viewToken == nil ? "detach \(url)" : "attach \(url)") }
  }

  func seek(fraction: Double) throws { places.log("seek \(fraction)") }

  func itemState(state: MediaState) throws { places.log("state \(state)") }

  func release() {
    let wasVisible = lock.withLock { () -> Bool in
      guard !released else { return false }
      released = true
      return last.visible
    }
    places.log("release \(url)")
    if wasVisible { places.note(visible: false, url: url) }
  }
}

final class RecordingWindow: WindowHandle, @unchecked Sendable {
  private let lock = NSLock()
  private var log: [String] = []
  private(set) var offMain = true
  private var readyListener: (any WindowReadyListener)?

  /// What the core does when the active page is prepared.
  func reportReady(_ ready: Bool) { lock.withLock { readyListener }?.changed(ready: ready) }

  var calls: [String] { lock.withLock { log } }

  var onlyOffMain: Bool { lock.withLock { offMain } }

  private func note(_ call: String) {
    lock.withLock {
      log.append(call)
      if Thread.isMainThread { offMain = false }
    }
  }

  func replace(active: UInt32, sources: [MediaItem?]) throws { note("replace \(active) \(sources.count)") }
  func replaceRows(rows: [[MediaItem]]) throws { note("rows \(rows.count)") }
  func showRows(first: UInt32, last: UInt32) throws { note("show \(first) \(last)") }
  func subscribeReady(listener: any WindowReadyListener) throws { lock.withLock { readyListener = listener } }
  func moveTo(active: UInt32) throws { note("move \(active)") }
  func setActive(isActive: Bool) throws { note("active \(isActive)") }
  func release() { note("release") }
}

struct ViewModelTests {
  @MainActor
  @Test
  func inv_audio_01_onlyTheEndedStateOfTheCoreStopsTheAudio() {
    let places = RecordingPlaces()
    let audio = AudioPlayback(item: TestAudio().item, playing: false, places: places)

    audio.toggle()
    audio.refresh(progress: 0.5)
    #expect(audio.progress == 0.5)
    audio.refresh(progress: 1)
    #expect(audio.isPlaying)
    audio.follow(.ended)

    #expect(!audio.isPlaying)
  }

  @MainActor
  @Test
  func inv_audio_06_everyEndRewindsToTheStartPausesAndReportsOnce() async {
    let places = RecordingPlaces()
    let audio = AudioPlayback(item: TestAudio().item, playing: true, places: places)
    audio.mountPlace()
    audio.refresh(progress: 0.7)

    #expect(audio.follow(.ended))
    #expect(!audio.isPlaying)
    #expect(audio.progress == 0)
    #expect(!audio.follow(.ended))
    await eventually("the first end sought to the start") { places.recorded.filter { $0 == "seek 0.0" }.count == 1 }
    audio.toggle()
    #expect(audio.follow(.ready) == false)
    audio.refresh(progress: 0.4)
    #expect(audio.follow(.ended))

    #expect(audio.progress == 0)
    await eventually("both ends sought to the start") { places.recorded.filter { $0 == "seek 0.0" }.count == 2 }
  }

  @MainActor
  @Test
  func inv_audio_07_theAudioStateTableDecidesSoundingBufferingAndFailed() {
    let table: [(MediaState, sounding: Bool, buffering: Bool, failed: Bool)] = [
      (.idle, false, true, false), (.loading, false, true, false), (.ready, true, false, false),
      (.rendered, true, false, false), (.ended, false, false, false), (.failed(detail: "x"), false, false, true)
    ]
    for (state, sounding, buffering, failed) in table {
      let audio = AudioPlayback(item: TestAudio().item, playing: true, places: RecordingPlaces())
      audio.follow(state)
      #expect(audio.isSounding == sounding, "\(state)")
      #expect(audio.isBuffering == buffering, "\(state)")
      #expect(state.isFailed == failed, "\(state)")
    }
    let paused = AudioPlayback(item: TestAudio().item, playing: false, places: RecordingPlaces())
    paused.follow(.rendered)
    #expect(!paused.isSounding && !paused.isBuffering)
  }

  @MainActor
  @Test
  func theAudioRateIsKeptAndSentToTheCore() {
    let places = RecordingPlaces()
    let audio = AudioPlayback(item: TestAudio().item, playing: false, rate: 1.5, places: places)
    #expect(audio.rate == 1.5)
    audio.mountPlace()

    audio.setRate(2)

    #expect(audio.rate == 2)
  }

  @Test
  func aVideoDeclaresItselfLoopingUnlessTheCallerAsksForOneShot() {
    let clip = TestClip()

    #expect(MediaVideo(clip).item.looping)
    #expect(!MediaVideo(clip, loop: false, onFirstFrame: {}, onEnd: {}).item.looping)
  }

  @Test
  func inv_mount_02_firstFrameFiresOncePerSourceAcrossLoadingAndEndFiresEveryTimeTheClipEnds() {
    var edges = VideoEdges()
    let steps: [(MediaState, firstFrame: Bool, ended: Bool)] = [
      (.loading, false, false), (.ready, false, false), (.rendered, true, false), (.rendered, false, false),
      (.ended, false, true), (.rendered, false, false), (.ended, false, true), (.loading, false, false), (.rendered, false, false), (.idle, false, false), (.rendered, true, false)
    ]

    for (state, firstFrame, ended) in steps {
      let reached = edges.advance(to: state)
      #expect((reached.firstFrame, reached.ended) == (firstFrame, ended), "\(state)")
    }
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

    let reported = Reported()
    _ = await MediaImagePipeline.load(TestImageSource(), request: request, pipeline: MediaImagePipeline.make(places: places), report: reported.record)

    #expect(reported.all.map { $0.hasPrefix("state failed") ? "state failed" : $0 } == expected)
  }

  @Test
  func aCancelledImageLoadDoesNotReportTheItemAsFailed() async {
    let places = RecordingPlaces()
    places.imageResult = .success(pngData(side: 8))
    let source = TestImageSource()
    let request = MediaImagePipeline.request(places: places, for: source)!
    let pipeline = MediaImagePipeline.make(places: places)

    let reported = Reported()
    let image = await Task {
      withUnsafeCurrentTask { $0?.cancel() }
      return await MediaImagePipeline.load(source, request: request, pipeline: pipeline, report: reported.record)
    }.value

    #expect(image == nil)
    #expect(!reported.all.contains { $0.hasPrefix("state failed") })
  }

  @Test
  func aCancelledLoadAnsweredFromTheMemoryCacheDeliversNothing() async {
    let places = RecordingPlaces()
    places.imageResult = .success(pngData(side: 8))
    let source = TestImageSource()
    let request = MediaImagePipeline.request(places: places, for: source)!
    let pipeline = MediaImagePipeline.make(places: places)
    let reported = Reported()
    let warm = await MediaImagePipeline.load(source, request: request, pipeline: pipeline, report: reported.record)
    #expect(warm != nil)
    let recorded = reported.all.count

    let image = await Task {
      withUnsafeCurrentTask { $0?.cancel() }
      return await MediaImagePipeline.load(source, request: request, pipeline: pipeline, report: reported.record)
    }.value

    #expect(image == nil)
    #expect(reported.all.count == recorded)
  }

  @Test
  func releaserDropsTheNukeMemoryCache() {
    let request = ImageRequest(url: URL(string: "https://o.test/cached.png"))
    ImageCache.shared[ImageCacheKey(request: request)] = ImageContainer(image: PlatformImage())
    #expect(ImageCache.shared[ImageCacheKey(request: request)] != nil)

    ImageMemoryReleaser(cache: .shared, frames: FrameCache()).dropMemoryCaches()

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

private final class Reported: @unchecked Sendable {
  private let lock = NSLock()
  private var states: [String] = []
  var all: [String] { lock.withLock { states } }
  var record: @Sendable (MediaState) -> Void { { state in self.lock.withLock { self.states.append("state \(state)") } } }
}

private struct TestAudio: MediaSource {
  var url: String { "note" }
  var kind: Kind { .audio }
  var headers: [String: String] { [:] }
}
