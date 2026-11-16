import Foundation

final class DemandBox: @unchecked Sendable {
  private let lock = NSLock()
  private var demand = AudioDemand()
  private var live = false
  private var listeners: [UUID: [UUID: AsyncStream<Bool>.Continuation]] = [:]
  private let focus: MediaAudioFocus

  init(focus: MediaAudioFocus) {
    self.focus = focus
  }

  func update(_ change: (inout AudioDemand) -> Bool) -> Bool {
    lock.withLock { change(&demand) }
  }

  var isRecording: Bool { lock.withLock { demand.recording } }

  var recordings: Int { lock.withLock { demand.recordings.count } }

  func changes(for id: UUID) -> AsyncStream<Bool> {
    AsyncStream { continuation in
      let listener = UUID()
      lock.withLock {
        guard demand.calls.contains(id) else { continuation.yield(false); continuation.finish(); return }
        listeners[id, default: [:]][listener] = continuation
        continuation.yield(live)
      }
      continuation.onTermination = { [weak self] _ in
        self?.lock.withLock { self?.listeners[id]?[listener] = nil }
      }
    }
  }

  func setLive(_ value: Bool) {
    let update: (Bool, [AsyncStream<Bool>.Continuation])? = lock.withLock {
      let next = value && !demand.calls.isEmpty
      guard live != next else { return nil }
      live = next
      let streams = listeners.filter { demand.calls.contains($0.key) }.flatMap { $0.value.values }
      return (next, streams)
    }
    guard let (next, streams) = update else { return }
    MediaTrace.emit("audio", "call live \(next)")
    streams.forEach { $0.yield(next) }
  }

  func removeCall(_ id: UUID) {
    let streams: [AsyncStream<Bool>.Continuation] = lock.withLock {
      demand.removeCall(id)
      let streams = listeners.removeValue(forKey: id).map { Array($0.values) } ?? []
      if demand.calls.isEmpty { live = false }
      return streams
    }
    MediaTrace.emit("audio", "call live false")
    streams.forEach { $0.yield(false); $0.finish() }
  }

  func target() -> SessionTarget {
    lock.withLock { demand.target(for: focus) }
  }
}
