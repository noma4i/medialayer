import Foundation
import Testing
@testable import MediaLayer
import MediaLayerTesting

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

private final class MemoryBody: BodyReader, @unchecked Sendable {
  private let lock = NSLock()
  private var rest: Data

  init(_ data: Data) { rest = data }

  func nextChunk() async throws -> Data? {
    lock.withLock {
      let count = min(rest.count, 4096)
      guard count > 0 else { return nil }
      defer { rest.removeFirst(count) }
      return Data(rest.prefix(count))
    }
  }

  func release() {}
}

private final class RangeIgnoringOrigin: Fetcher, @unchecked Sendable {
  static let payload = Data((0..<20_000).map { UInt8($0 % 251) })

  func fetch(request: FetchRequest) async throws -> FetchResponse {
    FetchResponse(status: 200, headers: ["Content-Type": "audio/mp4"], body: MemoryBody(Self.payload))
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

  func failed(error: MediaError) {}
}

struct MediaLayerBridgeTests {
  private func open(settings: MemorySettings) throws -> MediaLayer {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    return try MediaLayer.open(config: OpenConfig(
      dir: directory.path,
      availableBytes: UInt64.max, fetcher: TestFetcher(),
      settings: settings,
      slots: nil,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(),
      pool: defaultPoolSettings()
    ))
  }

  @Test
  func inv_api_17_a_typed_image_window_prefetches_the_remote_images_of_its_pages() async throws {
    let fetcher = TestFetcher()
    let media = try MediaLayer.open(config: OpenConfig(
      dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
      availableBytes: UInt64.max, fetcher: fetcher,
      settings: MemorySettings(),
      slots: nil,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(),
      pool: defaultPoolSettings(),
      imageWindows: [ImageWindow(name: "grid", behind: 0, from: 0, to: 2)]
    ))

    media.replaceImageWindow("grid", active: 0, pages: (0..<4).map { [Poster(url: "https://o.test/p\($0).jpg")] })

    for _ in 0..<200 where fetcher.requests.count < 2 { try await Task.sleep(for: .milliseconds(10)) }
    media.close()
    await media.awaitClosed()
    #expect(Set(fetcher.requests.map(\.url)) == ["https://o.test/p0.jpg", "https://o.test/p1.jpg"])
  }

  @Test
  func inv_image_47_the_image_size_reaches_the_caller_from_the_head_of_a_file() async throws {
    let media = try open(settings: MemorySettings())
    let file = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).png")
    let head: [UInt8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, 0x49, 0x48, 0x44, 0x52, 0, 0, 0x02, 0x80, 0, 0, 0x01, 0xe0, 8, 6, 0, 0, 0]
    try Data(head).write(to: file)
    defer { try? FileManager.default.removeItem(at: file) }

    let size = try await media.imageSize(Poster(url: file.absoluteString))

    #expect(size == ImageSize(width: 640, height: 480))
  }

  @Test
  func limitsReachThePlatformSettingsAndStatsAnswer() async throws {
    let settings = MemorySettings()
    let media = try open(settings: settings)
    let recorder = StatsRecorder()
    let subscription = try media.subscribeStats(listener: recorder)
    var limits = try await media.limits()
    limits.budget = 512 * 1024 * 1024
    limits.evict = .lfu

    try await media.setLimits(limits: limits)

    let stats = try await media.stats()
    #expect(try await media.limits() == limits)
    #expect(settings.keys == ["medialayer.limits"])
    #expect(stats.totalBytes == 0)
    #expect(stats.fileCount == 0)
    for _ in 0..<200 where recorder.totals.isEmpty { try await Task.sleep(for: .milliseconds(10)) }
    #expect(recorder.totals == [0])
    withExtendedLifetime(subscription) {}
  }

  @Test
  func inv_limits_08_reset_user_data_drops_the_cached_files_and_the_saved_limits() async throws {
    let settings = MemorySettings()
    let media = try MediaLayer.open(config: OpenConfig(
      dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
      availableBytes: UInt64.max, fetcher: RangeIgnoringOrigin(),
      settings: settings,
      slots: nil,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(),
      pool: defaultPoolSettings()
    ))
    var limits = try await media.limits()
    limits.budget = 512 * 1024 * 1024
    try await media.setLimits(limits: limits)
    _ = try await media.fetchImage(url: "https://o.test/user.jpg", headers: [:])
    #expect(try await media.stats().fileCount == 1)
    #expect(settings.keys == ["medialayer.limits"])

    try await media.resetUserData()

    #expect(try await media.stats().fileCount == 0)
    #expect(settings.keys.isEmpty)
    #expect(try await media.limits() == defaultLimits())
  }

  @Test
  func invalidLimitsThrowAMediaErrorWithDetail() async throws {
    let media = try open(settings: MemorySettings())
    var limits = defaultLimits()
    limits.budget = 1

    await #expect(throws: MediaError.self) { try await media.setLimits(limits: limits) }
  }

  @Test
  func inv_prefetch_16_theChunkPolicyAndTheByteBudgetCrossTheBridgeAndAWrongListIsRefusedWithItsKey() async throws {
    let limits = defaultLimits()
    #expect(limits.leadingChunks == [2, 1])
    #expect(limits.prefetchBudgetMib == 64)
    let parsed = try specDefaults(json: #"{"cache":{"prefetch_budget_mib":96},"videos":[{"name":"clip","prefetch":{"from":1,"to":4,"chunks":[2,2]}}]}"#)
    #expect(parsed.defaults.leadingChunks == [2, 2])
    #expect(parsed.defaults.prefetchBudgetMib == 96)
    #expect(throws: MediaError.self) { try specDefaults(json: #"{"videos":[{"name":"clip","prefetch":{"chunks":[1,2]}}]}"#) }

    let media = try open(settings: MemorySettings())
    var wrong = defaultLimits()
    wrong.leadingChunks = [3, 9]
    do {
      try await media.setLimits(limits: wrong)
      Issue.record("expected MediaError.invalidLimits")
    } catch MediaError.InvalidLimits(let detail) {
      #expect(detail.contains("leading_chunks"))
    }
  }

  @Test
  func statsStreamDeliversTheCurrentStats() async throws {
    let media = try open(settings: MemorySettings())
    var iterator = media.statsStream().makeAsyncIterator()

    try! await media.evictNow()

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
  func playUrlGoesThroughTheLocalProxyAndTheVideoPlaceholderIsTheFirstFrame() async throws {
    let media = try open(settings: MemorySettings())
    let source = MediaItem(kind: .video, url: "https://o.test/a.mp4", headers: [:], cover: nil, blurHash: nil, strategy: .window)

    let route = try await media.playUrl(source: source)

    #expect(route.url.hasPrefix("http://127.0.0.1:"))
    #expect(placeholderFor(source: source, ready: false) == .firstFrame)
  }

  @Test
  func aPlatformHttpClientReadsTheFetcherBytesThroughTheLoopbackProxy() async throws {
    let media = try MediaLayer.open(config: OpenConfig(
      dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
      availableBytes: UInt64.max, fetcher: RangeIgnoringOrigin(),
      settings: MemorySettings(),
      slots: nil,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(),
      pool: defaultPoolSettings()
    ))
    let route = try await media.playUrl(source: MediaItem(kind: .audio, url: "https://o.test/voice.m4a", headers: [:], cover: nil, blurHash: nil, strategy: .window))
    var request = URLRequest(url: URL(string: route.url)!)
    request.setValue("bytes=0-1", forHTTPHeaderField: "Range")

    let (data, response) = try await URLSession.shared.data(for: request)

    let http = try #require(response as? HTTPURLResponse)
    #expect(http.statusCode == 206)
    #expect(http.value(forHTTPHeaderField: "Content-Range") == "bytes 0-1/20000")
    #expect(data == RangeIgnoringOrigin.payload[0...1])
  }
}
