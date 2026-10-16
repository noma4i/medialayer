import Foundation
@testable import MediaLayerUI

final class FakeAudioSessionPort: AudioSessionPort, @unchecked Sendable {
  private let lock = NSLock()
  private var calls: [String] = []
  private var mainThreadCalls = 0
  private var busyDeactivations: Int

  init(busyDeactivations: Int = 0) {
    self.busyDeactivations = busyDeactivations
  }

  static func category(_ plan: SessionPlan) -> String {
    "category \(plan.category.rawValue) \(plan.mode.rawValue) \(plan.options.rawValue)"
  }

  var log: [String] { lock.withLock { calls } }
  var onMain: Int { lock.withLock { mainThreadCalls } }

  func setCategory(_ plan: SessionPlan) throws {
    record(Self.category(plan))
  }

  func setActive(_ active: Bool) throws {
    try lock.withLock {
      if !active && busyDeactivations > 0 {
        busyDeactivations -= 1
        calls.append("active false BUSY")
        throw NSError(domain: "fake", code: 1)
      }
    }
    record("active \(active)")
  }

  private func record(_ call: String) {
    lock.withLock {
      calls.append(call)
      if Thread.isMainThread { mainThreadCalls += 1 }
    }
  }
}
