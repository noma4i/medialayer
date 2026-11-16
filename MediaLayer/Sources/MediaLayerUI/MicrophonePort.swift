import Foundation

public enum MicrophoneAvailability: Sendable, Equatable {
  case available, ownCall, externalCall, otherApp, permissionDenied
}

struct MicrophoneState: Sendable, Equatable {
  var calls: Set<UUID> = []
  var inputAvailable = true
  var permissionGranted = true
  var interrupted = false
}

struct MicrophoneEvent: Sendable {
  let state: MicrophoneState
  let source: String
}

protocol MicrophonePort: Sendable {
  func current() -> MicrophoneState
  func changes() -> AsyncStream<String>
  func events() -> AsyncStream<MicrophoneEvent>
  func perform(_ action: @Sendable () -> Void)
  func observe(_ action: @escaping @Sendable (MicrophoneEvent) -> Void) -> Task<Void, Never>?
  func stop()
}

extension MicrophonePort {
  func stop() {}
  func perform(_ action: @Sendable () -> Void) { action() }
  func observe(_ action: @escaping @Sendable (MicrophoneEvent) -> Void) -> Task<Void, Never>? {
    let stream = events()
    return Task { for await event in stream { action(event) } }
  }
  func events() -> AsyncStream<MicrophoneEvent> {
    let changes = changes()
    return AsyncStream { continuation in
      let task = Task {
        for await source in changes { continuation.yield(MicrophoneEvent(state: current(), source: source)) }
      }
      continuation.onTermination = { _ in task.cancel() }
    }
  }
}

struct AvailableMicrophone: MicrophonePort {
  func current() -> MicrophoneState { MicrophoneState() }
  func changes() -> AsyncStream<String> { AsyncStream { _ in } }
}

final class MicrophoneStatus: @unchecked Sendable {
  private let lock = NSLock()
  private let delivery = NSRecursiveLock()
  private let port: MicrophonePort
  private let box: DemandBox
  private var own: [UUID: UUID?] = [:]
  private var failure = false
  private var value: MicrophoneAvailability = .available
  private var external = false
  private var sinks: [UUID: AsyncStream<MicrophoneAvailability>.Continuation] = [:]
  private var onExternal: (@Sendable (Bool) -> Void)?
  private var events: Task<Void, Never>?

  init(port: MicrophonePort, box: DemandBox) {
    self.port = port
    self.box = box
    events = port.observe { [weak self] event in self?.systemChanged(event.source, state: event.state) }
    refresh("subscribe")
  }

  deinit { events?.cancel() }

  func stop() {
    events?.cancel()
    port.stop()
  }

  var current: MicrophoneAvailability { lock.withLock { value } }

  func refreshForRecording() { refresh("recording") }

  var changes: AsyncStream<MicrophoneAvailability> {
    refresh("subscribe")
    return AsyncStream { continuation in
      let id = UUID()
      lock.withLock { sinks[id] = continuation; continuation.yield(value) }
      continuation.onTermination = { [weak self] _ in self?.lock.withLock { self?.sinks[id] = nil } }
    }
  }

  func setExternalListener(_ listener: @escaping @Sendable (Bool) -> Void) {
    delivery.withLock {
      let active = lock.withLock { onExternal = listener; return external }
      listener(active)
    }
  }

  func addCall(_ id: UUID, uuid: UUID?) {
    port.perform { [self] in
      lock.withLock {
        own[id] = .some(uuid)
        _ = box.update { demand in
          if demand.microphoneAvailability != .permissionDenied { demand.microphoneAvailability = .ownCall }
          return true
        }
      }
      apply("ownCall")
    }
  }

  func removeCall(_ id: UUID) {
    port.perform { [self] in
      lock.withLock { own[id] = nil }
      apply("ownCall")
    }
  }

  func refused() {
    port.perform { [self] in
      lock.withLock { failure = true }
      apply("recording refusal")
    }
  }

  func systemChanged(_ source: String, state: MicrophoneState? = nil) {
    if let state {
      lock.withLock { failure = false }
      apply(source, state: state)
    } else {
      port.perform { [self] in
        lock.withLock { failure = false }
        apply(source)
      }
    }
  }

  private func refresh(_ source: String, state snapshot: MicrophoneState? = nil) {
    port.perform { [self] in apply(source, state: snapshot) }
  }

  private func apply(_ source: String, state snapshot: MicrophoneState? = nil) {
    delivery.lock()
    defer { delivery.unlock() }
    let state = snapshot ?? port.current()
    let result: (MicrophoneAvailability?, [AsyncStream<MicrophoneAvailability>.Continuation], Bool?, (@Sendable (Bool) -> Void)?) = lock.withLock {
      let externalNow = !state.calls.subtracting(Set(own.values.compactMap { $0 })).isEmpty || (state.interrupted && own.isEmpty)
      let next: MicrophoneAvailability
      if !state.permissionGranted { next = .permissionDenied }
      else if !own.isEmpty { next = .ownCall }
      else if externalNow { next = .externalCall }
      else if !state.inputAvailable || failure { next = .otherApp }
      else { next = .available }
      let changed = next == value ? nil : next
      value = next
      _ = box.update { $0.microphoneAvailability = next; return true }
      let externalChange = external == externalNow ? nil : externalNow
      external = externalNow
      return (changed, Array(sinks.values), externalChange, onExternal)
    }
    if let changed = result.0 {
      MediaTrace.emit("audio", "microphone \(changed) source=\(source)")
      result.1.forEach { $0.yield(changed) }
    }
    if let active = result.2 { result.3?(active) }
  }
}
