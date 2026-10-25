import Foundation

private final class StatsRelay: StatsListener, @unchecked Sendable {
  private let continuation: AsyncStream<MediaStats>.Continuation

  init(_ continuation: AsyncStream<MediaStats>.Continuation) { self.continuation = continuation }

  func changed(stats: MediaStats) { continuation.yield(stats) }

  func failed(error: MediaError) { continuation.finish() }
}

private final class StateRelay: StateListener, @unchecked Sendable {
  private let continuation: AsyncStream<MediaState>.Continuation

  init(_ continuation: AsyncStream<MediaState>.Continuation) { self.continuation = continuation }

  func changed(state: MediaState) { continuation.yield(state) }

  func failed(error: MediaError) {
    continuation.yield(.failed(detail: String(describing: error)))
    continuation.finish()
  }
}

extension MediaLayer {
  public func imageSize(_ source: some MediaSource) async throws -> ImageSize? {
    try await imageSize(url: source.url, headers: source.headers)
  }

  public func statsStream() -> AsyncStream<MediaStats> {
    AsyncStream(bufferingPolicy: .bufferingNewest(1)) { continuation in
      do {
        let subscription = try subscribeStats(listener: StatsRelay(continuation))
        continuation.onTermination = { _ in withExtendedLifetime(subscription) {} }
      } catch {
        continuation.finish()
      }
    }
  }

  public func stateStream(key: String) -> AsyncStream<MediaState> {
    AsyncStream(bufferingPolicy: .bufferingNewest(1)) { continuation in
      do {
        let subscription = try subscribeState(key: key, listener: StateRelay(continuation))
        continuation.onTermination = { _ in withExtendedLifetime(subscription) {} }
      } catch {
        continuation.yield(.failed(detail: String(describing: error)))
        continuation.finish()
      }
    }
  }
}

extension MediaState {
  public var isFailed: Bool {
    if case .failed = self { true } else { false }
  }

  public var isReady: Bool {
    switch self {
    case .ready, .rendered, .ended: true
    case .idle, .loading, .failed: false
    }
  }
}
