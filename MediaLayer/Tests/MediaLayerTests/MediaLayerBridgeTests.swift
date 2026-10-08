import Foundation
import Testing
@testable import MediaLayer

private final class MemorySettings: SettingsStore, @unchecked Sendable {
  private let lock = NSLock()
  private var values: [String: String] = [:]

  var keys: [String] {
    lock.withLock { Array(values.keys) }
  }

  func read(key: String) -> String? { lock.withLock { values[key] } }
  func write(key: String, value: String) { lock.withLock { values[key] = value } }
  func remove(key: String) { lock.withLock { values[key] = nil } }
}

private final class OfflineFetcher: Fetcher, @unchecked Sendable {
  func fetch(request: FetchRequest) async throws -> FetchResponse {
    throw MediaError.Fetch(detail: "offline")
  }
}

private final class RecordingFetcher: Fetcher, @unchecked Sendable {
  private let lock = NSLock()
  private var seen: [String] = []

  var urls: [String] { lock.withLock { seen } }

  func fetch(request: FetchRequest) async throws -> FetchResponse {
    lock.withLock { seen.append(request.url) }
    throw MediaError.Fetch(detail: "offline")
  }
}

private struct Poster: MediaSource {
  let url: String
  var kind: Kind { .image }
  var headers: [String: String] { [:] }
}

private final class StatsRecorder: StatsListener, @unchecked Sendable {
  private let lock = NSLock()
  private var seen: [UInt64] = []

  var totals: [UInt64] { lock.withLock { seen } }

  func changed(stats: MediaStats) { lock.withLock { seen.append(stats.totalBytes) } }
}

struct MediaLayerBridgeTests {
  private func open(settings: MemorySettings) throws -> MediaLayer {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    return try MediaLayer.open(config: OpenConfig(
      dir: directory.path,
      fetcher: OfflineFetcher(),
      settings: settings,
      slots: nil,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(),
      pool: defaultPoolSettings()
    ))
  }

  @Test
  func inv_api_17_a_typed_image_window_prefetches_the_remote_images_of_its_pages() async throws {
    let fetcher = RecordingFetcher()
    let media = try MediaLayer.open(config: OpenConfig(
      dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
      fetcher: fetcher,
      settings: MemorySettings(),
      slots: nil,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(),
      pool: defaultPoolSettings(),
      imageWindows: [ImageWindow(name: "grid", behind: 0, from: 0, to: 2)]
    ))

    media.setImageWindow("grid", active: 0, pages: (0..<4).map { [Poster(url: "https://o.test/p\($0).jpg")] })

    for _ in 0..<200 where fetcher.urls.count < 2 { try await Task.sleep(for: .milliseconds(10)) }
    try await Task.sleep(for: .milliseconds(50))
    #expect(Set(fetcher.urls) == ["https://o.test/p0.jpg", "https://o.test/p1.jpg"])
  }

  @Test
  func limitsReachThePlatformSettingsAndStatsAnswer() async throws {
    let settings = MemorySettings()
    let media = try open(settings: settings)
    let recorder = StatsRecorder()
    let subscription = media.subscribeStats(listener: recorder)
    var limits = await media.limits()
    limits.budget = 512 * 1024 * 1024
    limits.evict = .lfu

    try await media.setLimits(limits: limits)

    let stats = await media.stats()
    #expect(await media.limits() == limits)
    #expect(settings.keys == ["medialayer.limits"])
    #expect(stats.totalBytes == 0)
    #expect(stats.fileCount == 0)
    for _ in 0..<200 where recorder.totals.isEmpty { try await Task.sleep(for: .milliseconds(10)) }
    #expect(recorder.totals == [0])
    withExtendedLifetime(subscription) {}
  }

  @Test
  func invalidLimitsThrowAMediaErrorWithDetail() async throws {
    let media = try open(settings: MemorySettings())
    var limits = defaultLimits()
    limits.budget = 1

    await #expect(throws: MediaError.self) { try await media.setLimits(limits: limits) }
  }

  @Test
  func statsStreamDeliversTheCurrentStats() async throws {
    let media = try open(settings: MemorySettings())
    var iterator = media.statsStream().makeAsyncIterator()

    await media.evictNow()

    let first = await iterator.next()
    #expect(first?.totalBytes == 0)
  }

  @Test
  func stateStreamStartsIdleForAnUndeclaredPlace() async throws {
    let media = try open(settings: MemorySettings())
    var iterator = media.stateStream(key: "https://o.test/a.mp4").makeAsyncIterator()

    let first = await iterator.next()

    #expect(first == .idle)
  }

  @Test
  func playUrlGoesThroughTheLocalProxyAndTheVideoPlaceholderIsTheFirstFrame() throws {
    let media = try open(settings: MemorySettings())
    let source = MediaItem(kind: .video, url: "https://o.test/a.mp4", headers: [:], cover: nil, blurHash: nil)

    let route = media.playUrl(source: source)

    #expect(route.url.hasPrefix("http://127.0.0.1:"))
    #expect(placeholderFor(source: source, cache: .missing, ready: false) == .firstFrame)
  }
}
