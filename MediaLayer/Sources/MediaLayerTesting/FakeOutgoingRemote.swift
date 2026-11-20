import Foundation
import MediaLayer
import MediaLayerUI

/// A remote for outgoing items: each url answers at once with what `answer` set, or waits until `complete` when it is held.
/// It counts the fetches in flight so a test can check the two-slot limit.
public final class FakeOutgoingRemote: OutgoingRemote, @unchecked Sendable {
  private let lock = NSLock()
  private var answers: [String: RemoteResult] = [:]
  private var held: Set<String> = []
  private var waiting: [String: [CheckedContinuation<RemoteResult, Never>]] = [:]
  private var asked: [String] = []
  private var running = 0
  private var peak = 0

  public init() {}

  public var requests: [String] { lock.withLock { asked } }
  public func requestCount(for url: String) -> Int { lock.withLock { asked.filter { $0 == url }.count } }
  public var inFlight: Int { lock.withLock { running } }
  public var peakInFlight: Int { lock.withLock { peak } }

  public func answer(_ url: String, _ result: RemoteResult) { lock.withLock { answers[url] = result } }

  /// The next fetches of `url` wait until `complete(_:with:)`.
  public func hold(_ url: String) { lock.withLock { _ = held.insert(url) } }

  public func complete(_ url: String, with result: RemoteResult = .complete) {
    let continuations = lock.withLock { () -> [CheckedContinuation<RemoteResult, Never>] in
      held.remove(url)
      answers[url] = result
      return waiting.removeValue(forKey: url) ?? []
    }
    for continuation in continuations { continuation.resume(returning: result) }
  }

  public func fetch(url: String, headers: [String: String]) async -> RemoteResult {
    lock.withLock {
      asked.append(url)
      running += 1
      peak = max(peak, running)
    }
    let result = await withCheckedContinuation { (continuation: CheckedContinuation<RemoteResult, Never>) in
      let immediate = lock.withLock { () -> RemoteResult? in
        if held.contains(url) {
          waiting[url, default: []].append(continuation)
          return nil
        }
        return answers[url] ?? .failed(status: 404, detail: "nothing served at \(url)")
      }
      if let immediate { continuation.resume(returning: immediate) }
    }
    lock.withLock { running -= 1 }
    return result
  }
}

/// The 5 s re-ask timer under a test's hand: a sleep ends only on `advance()`.
public final class ManualOutgoingClock: OutgoingClock, @unchecked Sendable {
  private let lock = NSLock()
  private var sleepers: [UInt64: CheckedContinuation<Void, Error>] = [:]
  private var next: UInt64 = 0
  private var asked: [UInt32] = []

  public init() {}

  public var sleeping: Int { lock.withLock { sleepers.count } }
  public var requestedMilliseconds: [UInt32] { lock.withLock { asked } }

  /// Ends every sleep that is waiting now.
  public func advance() {
    let ended = lock.withLock { () -> [CheckedContinuation<Void, Error>] in
      defer { sleepers = [:] }
      return Array(sleepers.values)
    }
    for continuation in ended { continuation.resume() }
  }

  public func sleep(milliseconds: UInt32) async throws {
    let id = lock.withLock { () -> UInt64 in
      next += 1
      asked.append(milliseconds)
      return next
    }
    try await withTaskCancellationHandler {
      try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
        lock.withLock { sleepers[id] = continuation }
      }
    } onCancel: {
      let continuation = lock.withLock { sleepers.removeValue(forKey: id) }
      continuation?.resume(throwing: CancellationError())
    }
  }
}
