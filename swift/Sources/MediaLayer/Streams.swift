import Foundation

private final class StatsRelay: StatsListener, @unchecked Sendable {
  private let continuation: AsyncStream<MediaStats>.Continuation

  init(_ continuation: AsyncStream<MediaStats>.Continuation) { self.continuation = continuation }

  func changed(stats: MediaStats) { continuation.yield(stats) }
}

private final class StateRelay: StateListener, @unchecked Sendable {
  private let continuation: AsyncStream<MediaState>.Continuation

  init(_ continuation: AsyncStream<MediaState>.Continuation) { self.continuation = continuation }

  func changed(state: MediaState) { continuation.yield(state) }
}

extension MediaLayer {
  public func statsStream() -> AsyncStream<MediaStats> {
    AsyncStream(bufferingPolicy: .bufferingNewest(1)) { continuation in
      let subscription = subscribeStats(listener: StatsRelay(continuation))
      continuation.onTermination = { _ in withExtendedLifetime(subscription) {} }
    }
  }

  public func stateStream(key: String) -> AsyncStream<MediaState> {
    AsyncStream(bufferingPolicy: .bufferingNewest(1)) { continuation in
      let subscription = subscribeState(key: key, listener: StateRelay(continuation))
      continuation.onTermination = { _ in withExtendedLifetime(subscription) {} }
    }
  }
}
