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
  let events = AsyncStream<UUID?>.makeStream()

  var requests: [Request] { lock.withLock { seen } }

  func fetch(request: FetchRequest) async throws -> FetchResponse {
    lock.withLock { seen.append(Request(url: request.url, start: request.rangeStart, end: request.rangeEnd)) }
    events.continuation.yield(nil)
    let start = Int(request.rangeStart ?? 0)
    let end = min(Int(request.rangeEnd ?? UInt64(Self.total - 1)), Self.total - 1)
    let length = end - start + 1
    var headers = ["Content-Type": "video/mp4", "Content-Length": String(length)]
    if request.rangeStart != nil { headers["Content-Range"] = "bytes \(start)-\(end)/\(Self.total)" }
    return FetchResponse(status: request.rangeStart == nil ? 200 : 206, headers: headers, body: MemoryBody(Data(repeating: 7, count: length)))
  }
}

private func eventually(_ origin: RangeOrigin, _ condition: String, _ holds: () -> Bool) async throws {
  let timer = DispatchSource.makeTimerSource()
  let timeoutID = UUID()
  timer.schedule(deadline: .now() + 8)
  timer.setEventHandler { origin.events.continuation.yield(timeoutID) }
  timer.resume()
  defer { timer.cancel() }
  var events = origin.events.stream.makeAsyncIterator()
  while !holds() {
    guard let event = await events.next() else { throw MediaError.Fetch(detail: "event stream ended: \(condition)") }
    if event == timeoutID && !holds() { throw MediaError.Fetch(detail: "timed out: \(condition)") }
  }
}

struct CacheStrategyTests {
  @Test
  func inv_strategy_15_a_whole_source_downloads_every_block_after_it_is_declared_and_hidden() async throws {
    let origin = RangeOrigin()
    let media = try MediaLayer.open(config: OpenConfig(
      dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
      fetcher: origin,
      settings: MemorySettings(),
      slots: nil,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(),
      pool: defaultPoolSettings()
    ))
    let clip = MediaItem(kind: .video, url: "https://o.test/clip.mp4", headers: [:], cover: nil, blurHash: nil, strategy: .whole)

    media.visible(source: clip, visible: true)
    try await eventually(origin, "first request") { !origin.requests.isEmpty }
    media.visible(source: clip, visible: false)

    try await eventually(origin, "all blocks") { origin.requests.count >= RangeOrigin.blocks.count }
    #expect(origin.requests == RangeOrigin.blocks)
    media.releaseOwned()
  }
}
