import Foundation
import MediaLayer
import Testing
@testable import MediaLayerUI

private final class OpenBody: BodyReader, @unchecked Sendable {
  private let finish = AsyncStream<Void>.makeStream()
  private let origin: HeldOrigin

  init(origin: HeldOrigin) { self.origin = origin }

  func nextChunk() async throws -> Data? {
    var completion = finish.stream.makeAsyncIterator()
    _ = await completion.next()
    return nil
  }

  func release() {
    finish.continuation.finish()
    origin.released()
  }
}

private final class HeldOrigin: Fetcher, @unchecked Sendable {
  private let lock = NSLock()
  private var urls: [String] = []
  private var releases = 0

  var releaseCount: Int { lock.withLock { releases } }

  func fetches(_ name: String) -> Int { lock.withLock { urls.filter { $0.contains(name) }.count } }

  var counters: String { "a=\(fetches("a.jpg")) b=\(fetches("b.jpg")) c=\(fetches("c.jpg")) released=\(releaseCount)" }

  func released() { lock.withLock { releases += 1 } }

  func fetch(request: FetchRequest) async throws -> FetchResponse {
    lock.withLock { urls.append(request.url) }
    return FetchResponse(status: 200, headers: [:], body: OpenBody(origin: self))
  }
}

private final class HoldSettings: SettingsStore, @unchecked Sendable {
  func read(key: String) -> String? { nil }
  func write(key: String, value: String) {}
  func remove(key: String) {}
}

private func photo(_ name: String) -> MediaItem {
  MediaItem(kind: .image, url: "https://o.test/\(name)", headers: [:], cover: nil, blurHash: nil, strategy: .whole)
}

private func waits(_ condition: () -> Bool) async -> Bool {
  for _ in 0..<400 {
    if condition() { return true }
    try? await Task.sleep(for: .milliseconds(25))
  }
  return condition()
}

@Suite(.timeLimit(.minutes(1)))
struct WholeHoldTests {
  @MainActor
  @Test
  func inv_strategy_21_a_hold_opens_adds_and_closes_the_declaration() async throws {
    let origin = HeldOrigin()
    let media = try MediaLayer.open(config: OpenConfig(
      dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
      availableBytes: UInt64.max, fetcher: origin, settings: HoldSettings(), slots: nil,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(), pool: defaultPoolSettings()
    ))
    let hold = MediaWholeHold()
    let (first, second) = (photo("a.jpg"), photo("b.jpg"))

    hold.declare([first], in: media)
    #expect(await waits { origin.fetches("a.jpg") >= 1 }, "\(origin.counters)")

    hold.declare([first, second], in: media)
    hold.declare([second], in: media)
    #expect(await waits { origin.releaseCount >= 1 && origin.fetches("b.jpg") >= 1 }, "\(origin.counters)")
    #expect(origin.fetches("a.jpg") == 1, "\(origin.counters)")

    hold.add([photo("c.jpg")], in: media)
    try? await Task.sleep(for: .milliseconds(300))
    #expect(origin.releaseCount == 1, "\(origin.counters)")
    #expect(origin.fetches("b.jpg") == 1, "\(origin.counters)")

    hold.release()
    #expect(await waits { origin.releaseCount >= 2 }, "\(origin.counters)")
    withExtendedLifetime(media) {}
  }
}
