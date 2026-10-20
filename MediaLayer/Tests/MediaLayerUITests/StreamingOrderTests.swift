import Foundation
import Testing
@testable import MediaLayerUI

private final class StreamOrder: @unchecked Sendable {
  let entered = AsyncStream<Void>.makeStream()
  let drained = DispatchSemaphore(value: 0)
  private let lock = NSLock()
  private var calls: [String] = []

  var actions: [String] { lock.withLock { calls } }

  func pause() {
    entered.continuation.yield(())
    _ = drained.wait(timeout: .now() + 5)
    lock.withLock { calls.append("pause") }
  }

  func resume() { lock.withLock { calls.append("resume") } }
}

struct StreamingOrderTests {
  @Test
  func drainingDuringPauseCompletesResumeAfterPause() async throws {
    let order = StreamOrder()
    let body = StreamingBody(limit: 10, pause: { order.pause() }, resume: { order.resume() }, cancel: {})
    let producer = Task.detached { body.receive(Data(count: 10)) }
    var entered = order.entered.stream.makeAsyncIterator()
    _ = await entered.next()
    let reader = Task.detached {
      let chunk = try await body.nextChunk()
      order.drained.signal()
      return chunk
    }
    #expect(try await reader.value?.count == 10)
    await producer.value
    #expect(order.actions == ["pause", "resume"])
  }
}
