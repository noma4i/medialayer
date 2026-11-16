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
  func stop()
}

extension MicrophonePort {
  func stop() {}
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
    let changes = port.events()
    refresh("subscribe")
    events = Task { [weak self] in
      for await event in changes { self?.systemChanged(event.source, state: event.state) }
    }
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
    delivery.lock()
    defer { delivery.unlock() }
    lock.withLock {
      own[id] = .some(uuid)
      _ = box.update { demand in
        if demand.microphoneAvailability != .permissionDenied { demand.microphoneAvailability = .ownCall }
        return true
      }
    }
    refresh("ownCall")
  }

  func removeCall(_ id: UUID) {
    delivery.lock()
    defer { delivery.unlock() }
    lock.withLock { own[id] = nil }
    refresh("ownCall")
  }

  func refused() {
    delivery.lock()
    defer { delivery.unlock() }
    lock.withLock { failure = true }
    refresh("recording refusal")
  }

  func systemChanged(_ source: String, state: MicrophoneState? = nil) {
    delivery.lock()
    defer { delivery.unlock() }
    lock.withLock { failure = false }
    refresh(source, state: state)
  }

  private func refresh(_ source: String, state snapshot: MicrophoneState? = nil) {
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
