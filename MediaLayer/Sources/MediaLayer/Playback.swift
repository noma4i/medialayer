import Foundation

private final class PlaybackValues: @unchecked Sendable {
  private let lock = NSLock()
  private var current = PlaybackState(playing: false, wish: false, reason: nil, refused: nil, attempt: 0)
  private var listeners: [UUID: AsyncStream<PlaybackState>.Continuation] = [:]
  private var finished = false
  var subscription: Subscription?

  deinit { listeners.values.forEach { $0.finish() } }

  var state: PlaybackState { lock.withLock { current } }

  func changed(state: PlaybackState) {
    lock.withLock {
      guard !finished else { return }
      current = state
      for listener in listeners.values { listener.yield(state) }
    }
  }

  func failed(error: MediaError) {
    NativeFailures.record(error)
    let finished = lock.withLock { () -> [AsyncStream<PlaybackState>.Continuation] in
      self.finished = true
      let finished = Array(listeners.values)
      listeners.removeAll()
      return finished
    }
    finished.forEach { $0.finish() }
  }

  func stream() -> AsyncStream<PlaybackState> {
    AsyncStream { continuation in
      let id = UUID()
      lock.withLock {
        guard !finished else { continuation.finish(); return }
        listeners[id] = continuation
        continuation.yield(current)
      }
      continuation.onTermination = { [weak self] _ in
        _ = self?.lock.withLock { self?.listeners.removeValue(forKey: id) }
      }
    }
  }
}

private final class PlaybackListenerRelay: PlaybackListener, @unchecked Sendable {
  weak var values: PlaybackValues?
  init(_ values: PlaybackValues) { self.values = values }
  func changed(state: PlaybackState) { values?.changed(state: state) }
  func failed(error: MediaError) { values?.failed(error: error) }
}

private enum PlaybackRegistry {
  static let lock = NSLock()
  static let values = NSMapTable<AnyObject, PlaybackValues>(keyOptions: .weakMemory, valueOptions: .strongMemory)

  static func values(for player: MediaPlayer) -> PlaybackValues {
    lock.withLock {
      if let values = values.object(forKey: player) { return values }
      let result = PlaybackValues()
      do { result.subscription = try player.subscribePlayback(listener: PlaybackListenerRelay(result)) }
      catch { result.failed(error: (error as? MediaError) ?? .Unexpected(detail: String(describing: error))) }
      values.setObject(result, forKey: player)
      return result
    }
  }
}

extension MediaPlayer {
  public var state: PlaybackState { PlaybackRegistry.values(for: self).state }
  public var states: AsyncStream<PlaybackState> { PlaybackRegistry.values(for: self).stream() }
}
