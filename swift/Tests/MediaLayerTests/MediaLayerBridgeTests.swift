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
