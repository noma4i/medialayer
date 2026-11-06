import Foundation
import MediaLayer

/// The call state of the process: active while at least one `MediaCallHold` lives. A recording registers while it runs, so the first hold reaches it
/// once and a recording that starts during a call is refused. No polling, no work at rest.
final class CallGate: @unchecked Sendable {
  private let lock = NSLock()
  private var holds = 0
  private var listeners: [UUID: @Sendable () -> Void] = [:]

  var inCall: Bool { lock.withLock { holds > 0 } }

  /// False while a call runs: the listener is not kept.
  func register(_ id: UUID, _ listener: @escaping @Sendable () -> Void) -> Bool {
    lock.withLock {
      guard holds == 0 else { return false }
      listeners[id] = listener
      return true
    }
  }

  func unregister(_ id: UUID) {
    lock.withLock { listeners[id] = nil }
  }

  func hold() {
    let reached = lock.withLock { () -> [@Sendable () -> Void] in
      holds += 1
      guard holds == 1 else { return [] }
      defer { listeners.removeAll() }
      return Array(listeners.values)
    }
    reached.forEach { $0() }
  }

  func release() {
    lock.withLock { if holds > 0 { holds -= 1 } }
  }
}

/// A call is running while this hold lives. Playback stops and `playUrl` is refused, a running `MediaVoiceRecorder` and a running video recording
/// of `MediaCamera` stop with the in-call refusal, and a new recording is refused. Releasing restores nothing: a place plays again when its owner declares play.
public final class MediaCallHold: @unchecked Sendable {
  private let core: CallHold
  private let gate: CallGate
  private let lock = NSLock()
  private var released = false

  init(core: CallHold, gate: CallGate) {
    self.core = core
    self.gate = gate
  }

  public func release() {
    guard lock.withLock({ defer { released = true }; return !released }) else { return }
    core.release()
    gate.release()
  }

  deinit {
    release()
  }
}

extension MediaLayer {
  /// Starts a call: the app holds this for as long as the call runs.
  public func callHold() throws -> MediaCallHold {
    try callHold(gate: Media.calls)
  }

  func callHold(gate: CallGate) throws -> MediaCallHold {
    let core = try holdCall()
    gate.hold()
    return MediaCallHold(core: core, gate: gate)
  }
}
