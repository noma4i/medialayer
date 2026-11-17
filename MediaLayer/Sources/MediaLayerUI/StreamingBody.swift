import Foundation
import MediaLayer

final class StreamingBody: BodyReader, @unchecked Sendable {
  static let chunkLimit = 64 * 1024
  private let lock = NSLock()
  private let limit: Int
  private let pause: @Sendable () -> Void
  private let resume: @Sendable () -> Void
  private let cancel: @Sendable () -> Void
  private var chunks: [Data?] = []
  private var readIndex = 0
  private var shifts = 0
  private var buffered = 0
  private var paused = false
  private var finished = false
  private var failure: Error?
  private var waiter: CheckedContinuation<Data?, Error>?

  var shiftedChunks: Int { lock.withLock { shifts } }

  init(
    limit: Int,
    pause: @escaping @Sendable () -> Void,
    resume: @escaping @Sendable () -> Void,
    cancel: @escaping @Sendable () -> Void
  ) {
    self.limit = limit
    self.pause = pause
    self.resume = resume
    self.cancel = cancel
  }

  deinit {
    cancel()
  }

  func release() {
    lock.lock()
    chunks = []
    readIndex = 0
    buffered = 0
    finished = true
    let waiter = self.waiter
    self.waiter = nil
    cancel()
    lock.unlock()
    waiter?.resume(returning: nil)
  }

  func receive(_ data: Data) {
    lock.lock()
    guard !finished else {
      lock.unlock()
      return
    }
    if let waiter {
      self.waiter = nil
      lock.unlock()
      waiter.resume(returning: data)
      return
    }
    chunks.append(data)
    buffered += data.count
    let shouldPause = buffered >= limit && !paused
    if shouldPause {
      paused = true
      pause()
    }
    lock.unlock()
  }

  func finish(_ error: Error?) {
    lock.lock()
    guard !finished else {
      lock.unlock()
      return
    }
    finished = true
    failure = error
    let waiter = self.waiter
    self.waiter = nil
    lock.unlock()
    guard let waiter else { return }
    if let error { waiter.resume(throwing: error) } else { waiter.resume(returning: nil) }
  }

  func nextChunk() async throws -> Data? {
    try await withTaskCancellationHandler {
      try await withCheckedThrowingContinuation { continuation in
        lock.lock()
        if readIndex < chunks.count, var chunk = chunks[readIndex] {
          chunks[readIndex] = nil
          readIndex += 1
          while readIndex < chunks.count, let next = chunks[readIndex], chunk.count + next.count <= Self.chunkLimit {
            chunk.append(next)
            chunks[readIndex] = nil
            readIndex += 1
          }
          if readIndex == chunks.count {
            chunks.removeAll(keepingCapacity: true)
            readIndex = 0
          } else if readIndex >= 64 && readIndex >= chunks.count / 2 {
            shifts += chunks.count - readIndex
            chunks.removeFirst(readIndex)
            readIndex = 0
          }
          buffered -= chunk.count
          let shouldResume = paused && buffered < limit / 2
          if shouldResume {
            paused = false
            resume()
          }
          lock.unlock()
          continuation.resume(returning: chunk)
        } else if finished {
          let error = failure
          lock.unlock()
          if let error { continuation.resume(throwing: error) } else { continuation.resume(returning: nil) }
        } else {
          waiter = continuation
          lock.unlock()
        }
      }
    } onCancel: {
      lock.withLock { cancel() }
    }
  }
}
