import Foundation
import MediaLayer
import Testing
@testable import MediaLayerUI

private final class OpenBody: BodyReader, @unchecked Sendable {
  private let finish = AsyncStream<Void>.makeStream()
  private let origin: HeldOrigin
  private let url: String

  init(origin: HeldOrigin, url: String) { self.origin = origin; self.url = url }

  func nextChunk() async throws -> Data? {
    origin.reading(url)
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
  let events = AsyncStream<String>.makeStream()

  var releaseCount: Int { lock.withLock { releases } }

  func fetches(_ name: String) -> Int { lock.withLock { urls.filter { $0.contains(name) }.count } }

  var counters: String { "a=\(fetches("a.jpg")) b=\(fetches("b.jpg")) c=\(fetches("c.jpg")) released=\(releaseCount)" }

  func reading(_ url: String) { events.continuation.yield(url); TestSignal.shared.fire() }
  func released() { lock.withLock { releases += 1 }; events.continuation.yield("released"); TestSignal.shared.fire() }

  func fetch(request: FetchRequest) async throws -> FetchResponse {
    lock.withLock { urls.append(request.url) }
    TestSignal.shared.fire()
    return FetchResponse(status: 200, headers: [:], body: OpenBody(origin: self, url: request.url))
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

@Suite(.timeLimit(.minutes(1)))
struct WholeHoldTests {
  @MainActor
  @Test
  func inv_strategy_21_a_hold_opens_adds_and_closes_the_declaration() async throws {
    let origin = HeldOrigin()
    var events = origin.events.stream.makeAsyncIterator()
    var limits = defaultLimits()
    limits.maxConcurrent = 3
    let media = try MediaLayer.open(config: OpenConfig(
      dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
      fetcher: origin, settings: HoldSettings(), slots: nil,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: limits, pool: defaultPoolSettings()
    ))
    let hold = MediaWholeHold()
    let (first, second) = (photo("a.jpg"), photo("b.jpg"))

    hold.declare([first], in: media)
    while let event = await events.next(), !event.contains("a.jpg") {}

    hold.declare([first, second], in: media)
    hold.declare([second], in: media)
    while origin.releaseCount < 1 || origin.fetches("b.jpg") < 1 { _ = await events.next() }
    #expect(origin.fetches("a.jpg") == 1, "\(origin.counters)")

    hold.add([photo("c.jpg")], in: media)
    while let event = await events.next(), !event.contains("c.jpg") {}
    #expect(origin.releaseCount == 1, "\(origin.counters)")
    #expect(origin.fetches("b.jpg") == 1, "\(origin.counters)")

    hold.release()
    while origin.releaseCount < 2 { _ = await events.next() }
    withExtendedLifetime(media) {}
  }
}
