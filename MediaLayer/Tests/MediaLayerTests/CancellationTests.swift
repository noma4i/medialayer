import Foundation
import Testing
@testable import MediaLayer

private final class PendingBody: BodyReader, @unchecked Sendable {
  let reading = AsyncStream<Void>.makeStream()
  let released = AsyncStream<Void>.makeStream()
  let finish = AsyncStream<Void>.makeStream()

  func nextChunk() async throws -> Data? {
    reading.continuation.yield(())
    var completion = finish.stream.makeAsyncIterator()
    _ = await completion.next()
    return nil
  }

  func release() {
    finish.continuation.finish()
    released.continuation.yield(())
    released.continuation.finish()
  }
}

private final class PendingFetcher: Fetcher, @unchecked Sendable {
  let body = PendingBody()

  func fetch(request: FetchRequest) async throws -> FetchResponse {
    FetchResponse(status: 200, headers: [:], body: body)
  }
}

private final class EmptySettings: SettingsStore {
  func read(key: String) -> String? { nil }
  func write(key: String, value: String) {}
  func remove(key: String) {}
}

@Suite(.timeLimit(.minutes(1)))
struct CancellationTests {
  private func arrives(_ stream: AsyncStream<Void>) async -> Bool {
    await withTaskGroup(of: Bool.self) { group in
      group.addTask {
        var events = stream.makeAsyncIterator()
        return await events.next() != nil
      }
      group.addTask {
        try? await Task.sleep(for: .seconds(3))
        return false
      }
      let result = await group.next()!
      group.cancelAll()
      return result
    }
  }

  @Test
  func inv_ffi_09_cancelling_outer_call_drops_inner_work() async throws {
    let fetcher = PendingFetcher()
    let media = try MediaLayer.open(config: OpenConfig(
      dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
      fetcher: fetcher, settings: EmptySettings(), slots: nil,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(), pool: defaultPoolSettings(), audioFocus: .exclusive
    ))
    let request = Task { try await media.fetchImage(url: "https://o.test/pending.jpg", headers: [:]) }
    #expect(await arrives(fetcher.body.reading.stream))
    request.cancel()
    #expect(await arrives(fetcher.body.released.stream), "cancelled image still owns the native body")
    fetcher.body.finish.continuation.finish()
    _ = await request.result
    withExtendedLifetime(media) {}
  }
}
