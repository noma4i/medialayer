import Foundation

private final class StatsRelay: StatsListener, @unchecked Sendable {
  private let continuation: AsyncThrowingStream<MediaStats, Error>.Continuation

  init(_ continuation: AsyncThrowingStream<MediaStats, Error>.Continuation) { self.continuation = continuation }

  func changed(stats: MediaStats) { continuation.yield(stats) }

  func failed(error: MediaError) {
    if case .Closed = error { continuation.finish() } else { continuation.finish(throwing: error) }
  }
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

private final class FailureRelay: FailureListener, @unchecked Sendable {
  private let continuation: AsyncStream<MediaFailure>.Continuation

  init(_ continuation: AsyncStream<MediaFailure>.Continuation) { self.continuation = continuation }

  func failed(failure: MediaFailure) { continuation.yield(failure) }
  func closed() { continuation.finish() }
}

public enum ToneUpdate: Sendable {
  case tone(MediaTone?)
  case failed(MediaError)
}

private final class ToneRelay: ToneListener, @unchecked Sendable {
  private let continuation: AsyncStream<ToneUpdate>.Continuation

  init(_ continuation: AsyncStream<ToneUpdate>.Continuation) { self.continuation = continuation }

  func changed(tone: MediaTone?) { continuation.yield(.tone(tone)) }

  func failed(error: MediaError) {
    continuation.yield(.failed(error))
    continuation.finish()
  }
}

extension MediaLayer {
  public func imageSize(_ source: some MediaSource) async throws -> ImageSize? {
    try await imageSize(url: source.url, headers: source.headers)
  }

  /// Every load failure and every cache store failure, one value each. Ending the iteration cancels the watch.
  public func failureStream() -> AsyncStream<MediaFailure> {
    AsyncStream(bufferingPolicy: .unbounded) { continuation in
      let subscription = subscribeFailures(listener: FailureRelay(continuation))
      continuation.onTermination = { _ in withExtendedLifetime(subscription) {} }
    }
  }

  public func statsStream() -> AsyncThrowingStream<MediaStats, Error> {
    AsyncThrowingStream(bufferingPolicy: .bufferingNewest(1)) { continuation in
      do {
        let subscription = try subscribeStats(listener: StatsRelay(continuation))
        continuation.onTermination = { _ in withExtendedLifetime(subscription) {} }
      } catch {
        if let error = error as? MediaError, case .Closed = error { continuation.finish() } else { continuation.finish(throwing: error) }
      }
    }
  }

  /// The tone of the source at `band`: the value after the watch is set, then after every change. Ending the iteration cancels the watch.
  public func toneStream(source: MediaItem, band: Float) -> AsyncStream<ToneUpdate> {
    AsyncStream(bufferingPolicy: .bufferingNewest(1)) { continuation in
      do {
        let subscription = try subscribeTone(source: source, band: band, listener: ToneRelay(continuation))
        continuation.onTermination = { _ in withExtendedLifetime(subscription) {} }
      } catch {
        continuation.yield(.failed((error as? MediaError) ?? .Unexpected(detail: String(describing: error))))
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

  public var isRendered: Bool {
    switch self {
    case .rendered, .ended: true
    case .idle, .loading, .ready, .failed: false
    }
  }
}
