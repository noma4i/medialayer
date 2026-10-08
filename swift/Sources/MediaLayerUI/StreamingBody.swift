import Foundation
import MediaLayer

final class StreamingBody: BodyReader, @unchecked Sendable {
  private let lock = NSLock()
  private let limit: Int
  private let pause: @Sendable () -> Void
  private let resume: @Sendable () -> Void
  private let cancel: @Sendable () -> Void
  private var chunks: [Data] = []
  private var buffered = 0
  private var paused = false
  private var finished = false
  private var failure: Error?
  private var waiter: CheckedContinuation<Data?, Error>?

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
    buffered = 0
    finished = true
    let waiter = self.waiter
    self.waiter = nil
    lock.unlock()
    cancel()
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
    if shouldPause { paused = true }
    lock.unlock()
    if shouldPause { pause() }
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
        if !chunks.isEmpty {
          let chunk = chunks.removeFirst()
          buffered -= chunk.count
          let shouldResume = paused && buffered < limit / 2
          if shouldResume { paused = false }
          lock.unlock()
          if shouldResume { resume() }
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
      cancel()
    }
  }
}
