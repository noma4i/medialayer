import Foundation
import Testing
@testable import MediaLayer

private struct Request: Equatable {
  let url: String
  let start: UInt64?
  let end: UInt64?
}

private final class MemorySettings: SettingsStore, @unchecked Sendable {
  func read(key: String) -> String? { nil }
  func write(key: String, value: String) {}
  func remove(key: String) {}
}

private final class MemoryBody: BodyReader, @unchecked Sendable {
  private let lock = NSLock()
  private var rest: Data

  init(_ data: Data) { rest = data }

  func nextChunk() async throws -> Data? {
    lock.withLock {
      let count = min(rest.count, 65_536)
      guard count > 0 else { return nil }
      defer { rest.removeFirst(count) }
      return Data(rest.prefix(count))
    }
  }

  func release() {}
}

private final class RangeOrigin: Fetcher, @unchecked Sendable {
  static let total = 2 * 1024 * 1024 + 512 * 1024
  static let blocks: [Request] = [(0, 1_048_575), (1_048_576, 2_097_151), (2_097_152, 2_621_439)].map {
    Request(url: "https://o.test/clip.mp4", start: UInt64($0.0), end: UInt64($0.1))
  }

  private let lock = NSLock()
  private var seen: [Request] = []
  private let refusal: @Sendable (Request, Int) -> Bool

  init(refusal: @escaping @Sendable (Request, Int) -> Bool = { _, _ in false }) { self.refusal = refusal }

  var requests: [Request] { lock.withLock { seen } }

  func fetch(request: FetchRequest) async throws -> FetchResponse {
    let asked = Request(url: request.url, start: request.rangeStart, end: request.rangeEnd)
    let attempt = lock.withLock { () -> Int in
      seen.append(asked)
      return seen.filter { $0 == asked }.count
    }
    if refusal(asked, attempt) { throw MediaError.Fetch(detail: "cut") }
    let start = Int(request.rangeStart ?? 0)
    let end = min(Int(request.rangeEnd ?? UInt64(Self.total - 1)), Self.total - 1)
    let length = end - start + 1
    var headers = ["Content-Type": "video/mp4", "Content-Length": String(length)]
    if request.rangeStart != nil { headers["Content-Range"] = "bytes \(start)-\(end)/\(Self.total)" }
    return FetchResponse(status: request.rangeStart == nil ? 200 : 206, headers: headers, body: MemoryBody(Data(repeating: 7, count: length)))
  }
}

private func item(_ url: String, _ kind: Kind = .video, _ strategy: CacheStrategy) -> MediaItem {
  MediaItem(kind: kind, url: url, headers: [:], cover: nil, blurHash: nil, strategy: strategy)
}

private func eventually(timeout: Double = 8, _ condition: () -> Bool) async throws {
  let deadline = Date().addingTimeInterval(timeout)
  while !condition() {
    guard Date() < deadline else { throw MediaError.Fetch(detail: "condition not reached in time") }
    try await Task.sleep(for: .milliseconds(10))
  }
}

struct CacheStrategyTests {
  private func open(_ fetcher: Fetcher, directory: URL = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)) throws -> MediaLayer {
    try MediaLayer.open(config: OpenConfig(
      dir: directory.path,
      availableBytes: UInt64.max, fetcher: fetcher,
      settings: MemorySettings(),
      slots: nil,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(),
      pool: defaultPoolSettings()
    ))
  }

  @Test
  func inv_strategy_15_a_whole_source_downloads_every_block_after_it_is_declared_and_hidden() async throws {
    let origin = RangeOrigin()
    let media = try open(origin)
    let clip = item("https://o.test/clip.mp4", .video, .whole)

    media.visible(source: clip, visible: true)
    media.visible(source: clip, visible: false)

    try await eventually { Set(origin.requests.map { "\($0.start ?? 0)" }) == Set(RangeOrigin.blocks.map { "\($0.start ?? 0)" }) }
    #expect(origin.requests == RangeOrigin.blocks)
  }

  @Test
  func inv_strategy_15_a_whole_source_far_from_the_window_downloads_in_full() async throws {
    let origin = RangeOrigin()
    let media = try open(origin)
    var list: [MediaItem?] = (0..<40).map { item("https://o.test/other\($0).mp4", .video, .window) }
    list[35] = item("https://o.test/clip.mp4", .video, .whole)

    media.replaceWindow(active: 0, sources: list)

    try await eventually { origin.requests.filter { $0.url == "https://o.test/clip.mp4" }.count >= 3 }
    #expect(origin.requests.filter { $0.url == "https://o.test/clip.mp4" } == RangeOrigin.blocks)
  }

  @Test
  func inv_strategy_15_the_image_window_skips_on_demand_sources() async throws {
    let origin = RangeOrigin()
    let media = try open(origin)

    media.replaceImageWindow(name: "feed", active: 0, pages: (0..<6).map { [item("https://o.test/p\($0).jpg", .image, .onDemand)] })
    try await Task.sleep(for: .milliseconds(400))

    #expect(origin.requests.isEmpty)
  }

  @Test
  func inv_strategy_15_a_local_whole_source_is_never_fetched() async throws {
    let origin = RangeOrigin()
    let media = try open(origin)
    let local = item("file:///bundle/intro.mp4", .video, .whole)

    media.visible(source: local, visible: true)
    media.replaceWindow(active: 0, sources: [local])
    try await Task.sleep(for: .milliseconds(400))

    #expect(origin.requests.isEmpty)
  }

  @Test
  func inv_strategy_15_a_failed_block_is_retried_and_the_download_completes() async throws {
    let origin = RangeOrigin { request, attempt in request.start == 1_048_576 && attempt <= 2 }
    let media = try open(origin)

    media.visible(source: item("https://o.test/clip.mp4", .video, .whole), visible: true)

    try await eventually(timeout: 15) { origin.requests.filter { $0.start == 2_097_152 }.count == 1 }
    #expect(origin.requests.filter { $0.start == 1_048_576 }.count == 3)
    #expect(origin.requests.filter { $0.start == 0 }.count == 1)
  }

  @Test
  func inv_strategy_15_a_download_cut_by_a_restart_resumes_at_the_first_missing_block() async throws {
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let cut = RangeOrigin { request, _ in (request.start ?? 0) >= 2_097_152 }
    var first: MediaLayer? = try open(cut, directory: directory)
    first?.visible(source: item("https://o.test/clip.mp4", .video, .whole), visible: true)
    try await eventually(timeout: 15) { cut.requests.filter { $0.start == 2_097_152 }.count == 3 }
    try await Task.sleep(for: .milliseconds(2_000))
    first = nil

    let healthy = RangeOrigin()
    let second = try open(healthy, directory: directory)
    second.visible(source: item("https://o.test/clip.mp4", .video, .whole), visible: true)

    try await eventually { healthy.requests.count >= 1 }
    try await Task.sleep(for: .milliseconds(300))
    #expect(healthy.requests == [RangeOrigin.blocks[2]])
  }
}
