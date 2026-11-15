import Foundation
import MediaLayer
import Testing

final class PlayerEventLog: PlayerEventsProtocol, @unchecked Sendable {
  private let lock = NSLock()
  private var recorded: [String] = []
  private var memory: [Pressure] = []
  private var stamped: [(name: String, generation: UInt64)] = []
  private var heldReady: Set<UInt64> = []
  private var parkedReady: [UInt64] = []
  private var waiters: [(id: UUID, name: String, continuation: CheckedContinuation<Void, Never>)] = []

  var all: [String] { lock.withLock { recorded } }
  var pressures: [Pressure] { lock.withLock { memory } }
  func generations(_ name: String) -> [UInt64] { lock.withLock { stamped.filter { $0.name == name }.map(\.generation) } }

  func count(_ name: String) -> Int { all.filter { $0 == name }.count }

  func wait(for name: String, timeout: Duration = .seconds(10)) async {
    let id = UUID()
    let timer = Task { [self] in
      try? await Task.sleep(for: timeout)
      guard !Task.isCancelled, let waiter = lock.withLock({ () -> CheckedContinuation<Void, Never>? in
        guard let index = waiters.firstIndex(where: { $0.id == id }) else { return nil }
        return waiters.remove(at: index).continuation
      }) else { return }

      Issue.record("player event \"\(name)\" never arrived within \(timeout)")
      waiter.resume()
    }
    await withCheckedContinuation { continuation in
      let present = lock.withLock { () -> Bool in
        let present = recorded.contains(name)
        if !present { waiters.append((id, name, continuation)) }
        return present
      }
      if present { continuation.resume() }
    }
    timer.cancel()
  }

  func background() { record("background") }
  func foreground() { record("foreground") }
  func memoryWarning(level: Pressure) {
    lock.withLock { memory.append(level) }
    record("memoryWarning")
  }
  func attach(key: String, viewToken: UInt64) { record("attach \(key)") }
  func detach(key: String) { record("detach \(key)") }
  func ready(slot: UInt32, generation: UInt64) {
    let held = lock.withLock { () -> Bool in
      guard heldReady.contains(generation) else { return false }
      parkedReady.append(generation)
      return true
    }
    TestSignal.shared.fire()
    if !held { stamp("ready", generation); record("ready") }
  }

  /// Parks the `ready` of one generation until `releaseReady`, so a test controls when it reaches the log.
  func holdReady(generation: UInt64) { lock.withLock { _ = heldReady.insert(generation) } }

  func isParked(generation: UInt64) -> Bool { lock.withLock { parkedReady.contains(generation) } }

  func releaseReady(generation: UInt64) {
    let parked = lock.withLock { () -> Bool in
      heldReady.remove(generation)
      guard let index = parkedReady.firstIndex(of: generation) else { return false }
      parkedReady.remove(at: index)
      return true
    }
    if parked { stamp("ready", generation); record("ready") }
  }
  func visualReady(slot: UInt32, generation: UInt64) { stamp("visualReady", generation); record("visualReady") }
  func stalled(slot: UInt32, generation: UInt64, positionMs: UInt64) { record("stalled") }
  func failed(slot: UInt32, generation: UInt64, status: Int32, detail: String) { stamp("failed", generation); record("failed \(status)") }
  func ended(slot: UInt32, generation: UInt64) { stamp("ended", generation); record("ended") }

  private func stamp(_ name: String, _ generation: UInt64) { lock.withLock { stamped.append((name, generation)) } }

  private func record(_ name: String) {
    let matched = lock.withLock { () -> [CheckedContinuation<Void, Never>] in
      recorded.append(name)
      let matched = waiters.filter { $0.name == name }.map(\.continuation)
      waiters.removeAll { $0.name == name }
      return matched
    }
    matched.forEach { $0.resume() }
    TestSignal.shared.fire()
  }
}
