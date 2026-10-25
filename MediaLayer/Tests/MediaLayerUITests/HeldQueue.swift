import Foundation

/// Occupies every worker of one global queue so a block submitted after `hold` runs only after `release`.
final class HeldQueue: @unchecked Sendable {
  private let gate = DispatchSemaphore(value: 0)
  private let blockers: Int

  init(qos: DispatchQoS.QoSClass = .userInitiated, blockers: Int = 1500) {
    self.blockers = blockers
    for _ in 0 ..< blockers { DispatchQueue.global(qos: qos).async { [gate] in gate.wait() } }
  }

  func release() { for _ in 0 ..< blockers { gate.signal() } }
}
