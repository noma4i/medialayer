import Darwin
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

private func closeListenerSilently(port: UInt16) {
  for descriptor in 3..<4096 {
    var address = sockaddr_in()
    var length = socklen_t(MemoryLayout<sockaddr_in>.size)
    let named = withUnsafeMutablePointer(to: &address) { $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { getsockname(Int32(descriptor), $0, &length) } }
    var peer = sockaddr_in()
    var peerLength = socklen_t(MemoryLayout<sockaddr_in>.size)
    let connected = withUnsafeMutablePointer(to: &peer) { $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { getpeername(Int32(descriptor), $0, &peerLength) } } == 0
    guard named == 0, !connected, UInt16(bigEndian: address.sin_port) == port else { continue }
    let null = open("/dev/null", O_RDONLY)
    dup2(null, Int32(descriptor))
    close(null)
    return
  }
  Issue.record("no listening socket on port \(port)")
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

    media.replaceImageWindow("grid", active: 0, pages: (0..<4).map { [Poster(url: "https://o.test/p\($0).jpg")] })

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
  func inv_prefetch_16_theChunkPolicyAndTheByteBudgetCrossTheBridgeAndAWrongListIsRefusedWithItsKey() async throws {
    let limits = defaultLimits()
    #expect(limits.leadingChunks == [3, 2, 1])
    #expect(limits.prefetchBudgetMib == 64)
    let parsed = try specDefaults(json: #"{"cache":{"prefetch_budget_mib":96},"videos":[{"name":"clip","prefetch":{"from":1,"to":4,"chunks":[4,2]}}]}"#)
    #expect(parsed.defaults.leadingChunks == [4, 2])
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
  func playUrlGoesThroughTheLocalProxyAndTheVideoPlaceholderIsTheFirstFrame() async throws {
    let media = try open(settings: MemorySettings())
    let source = MediaItem(kind: .video, url: "https://o.test/a.mp4", headers: [:], cover: nil, blurHash: nil)

    let route = try await media.playUrl(source: source)

    #expect(route.url.hasPrefix("http://127.0.0.1:"))
    #expect(placeholderFor(source: source, cache: .missing, ready: false) == .firstFrame)
  }

  @Test
  func inv_proxy_22_an_origin_that_ignores_range_and_sends_no_length_still_answers_a_player_range_with_206() async throws {
    let media = try MediaLayer.open(config: OpenConfig(
      dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
      fetcher: RangeIgnoringOrigin(),
      settings: MemorySettings(),
      slots: nil,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(),
      pool: defaultPoolSettings()
    ))
    let route = try await media.playUrl(source: MediaItem(kind: .audio, url: "https://o.test/voice.m4a", headers: [:], cover: nil, blurHash: nil))
    let payload = RangeIgnoringOrigin.payload

    for (range, bytes) in [("bytes=0-1", 0...1), ("bytes=-100", 19_900...19_999), ("bytes=10000-", 10_000...19_999)] {
      var request = URLRequest(url: URL(string: route.url)!)
      request.setValue(range, forHTTPHeaderField: "Range")
      let (data, response) = try await URLSession.shared.data(for: request)
      let http = try #require(response as? HTTPURLResponse)
      #expect(http.statusCode == 206, "\(range)")
      #expect(http.value(forHTTPHeaderField: "Content-Range") == "bytes \(bytes.lowerBound)-\(bytes.upperBound)/20000", "\(range)")
      #expect(data == payload[bytes], "\(range)")
    }
  }

  @Test
  func inv_proxy_23_a_listener_the_system_closed_is_bound_again_when_the_app_returns_to_the_foreground() async throws {
    let media = try MediaLayer.open(config: OpenConfig(
      dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
      fetcher: RangeIgnoringOrigin(),
      settings: MemorySettings(),
      slots: nil,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(),
      pool: defaultPoolSettings()
    ))
    let route = try await media.playUrl(source: MediaItem(kind: .audio, url: "https://o.test/voice.m4a", headers: [:], cover: nil, blurHash: nil))
    let url = try #require(URL(string: route.url))
    closeListenerSilently(port: UInt16(try #require(url.port)))
    await #expect(throws: (any Error).self) { _ = try await URLSession.shared.data(from: url) }

    media.playerEvents().foreground()

    var status = 0
    for _ in 0..<100 where status != 200 {
      if let (_, response) = try? await URLSession.shared.data(from: url) { status = (response as? HTTPURLResponse)?.statusCode ?? 0 }
      try await Task.sleep(nanoseconds: 50_000_000)
    }
    #expect(status == 200)
  }
}
