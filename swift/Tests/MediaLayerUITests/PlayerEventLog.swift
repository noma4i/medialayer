import Foundation
import MediaLayer

final class PlayerEventLog: PlayerEventsProtocol, @unchecked Sendable {
  private let lock = NSLock()
  private var recorded: [String] = []
  private var memory: [Pressure] = []
  private var waiters: [(String, CheckedContinuation<Void, Never>)] = []

  var all: [String] { lock.withLock { recorded } }
  var pressures: [Pressure] { lock.withLock { memory } }

  func count(_ name: String) -> Int { all.filter { $0 == name }.count }

  func wait(for name: String) async {
    await withCheckedContinuation { continuation in
      let present = lock.withLock { () -> Bool in
        let present = recorded.contains(name)
        if !present { waiters.append((name, continuation)) }
        return present
      }
      if present { continuation.resume() }
    }
  }

  func background() { record("background") }
  func foreground() { record("foreground") }
  func memoryWarning(level: Pressure) {
    lock.withLock { memory.append(level) }
    record("memoryWarning")
  }
  func attach(key: String, viewToken: UInt64) { record("attach \(key)") }
  func detach(key: String) { record("detach \(key)") }
  func ready(slot: UInt32) { record("ready") }
  func visualReady(slot: UInt32) { record("visualReady") }
  func stalled(slot: UInt32, positionMs: UInt64) { record("stalled") }
  func failed(slot: UInt32, status: Int32, detail: String) { record("failed \(status)") }
  func ended(slot: UInt32) { record("ended") }

  private func record(_ name: String) {
    let matched = lock.withLock { () -> [CheckedContinuation<Void, Never>] in
      recorded.append(name)
      let matched = waiters.filter { $0.0 == name }.map(\.1)
      waiters.removeAll { $0.0 == name }
      return matched
    }
    matched.forEach { $0.resume() }
  }
}
