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
  func inv_memory_07_read_chunks_release_their_storage_before_compaction() async throws {
    let released = ReleasedChunks()
    let body = StreamingBody(limit: 1 << 20, pause: {}, resume: {}, cancel: {})
    for _ in 0..<128 {
      let memory = UnsafeMutableRawPointer.allocate(byteCount: 1024, alignment: 16)
      body.receive(Data(bytesNoCopy: memory, count: 1024, deallocator: .custom { pointer, _ in
        pointer.deallocate()
        released.record()
      }))
    }
    for _ in 0..<32 { _ = try await body.nextChunk() }
    #expect(released.count == 32)
    body.release()
    #expect(released.count == 128)
  }

  @Test
  func inv_proxy_06_chunk_queue_moves_at_most_linear_elements() async throws {
    let body = StreamingBody(limit: 8192, pause: {}, resume: {}, cancel: {})
    for index in 0 ..< 4096 { body.receive(Data([UInt8(index % 256)])) }
    body.finish(nil)
    for index in 0 ..< 4096 {
      #expect(try await body.nextChunk() == Data([UInt8(index % 256)]))
    }
    #expect(try await body.nextChunk() == nil)
    #expect(body.shiftedChunks <= 4096)
  }

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

private final class ReleasedChunks: @unchecked Sendable {
  private let lock = NSLock()
  private var calls = 0
  var count: Int { lock.withLock { calls } }
  func record() { lock.withLock { calls += 1 } }
}
