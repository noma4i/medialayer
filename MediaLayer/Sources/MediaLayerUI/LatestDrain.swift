import Foundation

/// Runs `apply` off the caller with at most one running block and one pending value per drain; later values merge into the pending one.
/// Each drain schedules on the shared concurrent queue, so a held `apply` of one drain never delays another drain.
/// A value that a merge replaced is kept in `retired` and released inside the drain block, never on the submitting thread.
final class LatestDrain<Value: Sendable>: @unchecked Sendable {
  private let lock = NSLock()
  private var pending: Value?
  private var retired: [Value] = []
  private var isScheduled = false
  private var idleWaiters: [CheckedContinuation<Void, Never>] = []
  private let merge: @Sendable (Value?, Value) -> Value
  private let apply: @Sendable (Value) -> Void

  init(merge: @escaping @Sendable (Value?, Value) -> Value, apply: @escaping @Sendable (Value) -> Void) {
    self.merge = merge
    self.apply = apply
  }

  func submit(_ value: Value) {
    let schedules = lock.withLock { () -> Bool in
      let replaced = pending
      pending = merge(replaced, value)
      if let replaced { retired.append(replaced) }
      if isScheduled { return false }
      isScheduled = true
      return true
    }
    if schedules { schedule() }
  }

  func idle() async {
    await withCheckedContinuation { continuation in
      let ready = lock.withLock {
        if !isScheduled { return true }
        idleWaiters.append(continuation)
        return false
      }
      if ready { continuation.resume() }
    }
  }

  private func schedule() {
    DispatchQueue.global(qos: .userInitiated).async { self.run() }
  }

  /// One apply per block. A value that arrived meanwhile gets its own block, so the executor is released between applies.
  private func run() {
    var (next, dead) = lock.withLock { () -> (Value?, [Value]) in
      defer {
        pending = nil
        retired = []
      }
      return (pending, retired)
    }
    dead.removeAll()
    if let next { apply(next) }
    next = nil
    let (again, released, waiters) = lock.withLock { () -> (Bool, [Value], [CheckedContinuation<Void, Never>]) in
      defer { retired = [] }
      if pending == nil { isScheduled = false }
      let waiters = isScheduled ? [] : idleWaiters
      if !isScheduled { idleWaiters = [] }
      return (pending != nil, retired, waiters)
    }
    var late = released
    late.removeAll()
    waiters.forEach { $0.resume() }
    if again { schedule() }
  }
}
