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
    let body = StreamingBody(limit: 16 << 20, pause: {}, resume: {}, cancel: {})
    for _ in 0..<128 {
      let memory = UnsafeMutableRawPointer.allocate(byteCount: 65536, alignment: 16)
      body.receive(Data(bytesNoCopy: memory, count: 65536, deallocator: .custom { pointer, _ in
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
    let body = StreamingBody(limit: 1 << 20, pause: {}, resume: {}, cancel: {})
    let count = 4 * 65536 + 100
    for index in 0 ..< count { body.receive(Data([UInt8(index % 256)])) }
    body.finish(nil)
    var read = 0
    while let chunk = try await body.nextChunk() { read += chunk.count }
    #expect(read == count)
    #expect(body.shiftedChunks <= count)
  }

  @Test
  func inv_proxy_44_ready_body_bytes_cross_in_64_kib_chunks() async throws {
    let body = StreamingBody(limit: 2 << 20, pause: {}, resume: {}, cancel: {})
    let payload = Data((0 ..< (1 << 20)).map { UInt8($0 % 251) })
    for start in stride(from: 0, to: payload.count, by: 8192) { body.receive(payload[start ..< start + 8192]) }
    body.finish(nil)
    var chunks: [Data] = []
    while let chunk = try await body.nextChunk() { chunks.append(chunk) }
    #expect(chunks.map(\.count) == Array(repeating: 65536, count: 16))
    #expect(chunks.reduce(Data(), +) == payload)
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
