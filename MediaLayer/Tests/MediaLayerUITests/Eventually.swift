import Foundation
import ObjectiveC
import Testing
@testable import MediaLayerUI

final class TestSignal: @unchecked Sendable {
  static let shared = TestSignal()
  private let lock = NSLock()
  private var generation: UInt64 = 0
  private var waiters: [UUID: CheckedContinuation<Bool, Never>] = [:]

  var current: UInt64 { lock.withLock { generation } }

  func fire() {
    let pending = lock.withLock { () -> [CheckedContinuation<Bool, Never>] in
      generation &+= 1
      let pending = Array(waiters.values)
      waiters.removeAll()
      return pending
    }
    pending.forEach { $0.resume(returning: true) }
  }

  func changed(since previous: UInt64, deadline: DispatchTime) async -> Bool {
    let id = UUID()
    let timer = DispatchSource.makeTimerSource()
    timer.schedule(deadline: deadline)
    timer.setEventHandler { [self] in
      let waiter = lock.withLock { waiters.removeValue(forKey: id) }
      waiter?.resume(returning: false)
    }
    timer.resume()
    defer { timer.cancel() }
    return await withCheckedContinuation { continuation in
      let result = lock.withLock { () -> Bool? in
        guard generation == previous else { return true }
        guard DispatchTime.now() < deadline else { return false }
        waiters[id] = continuation
        return nil
      }
      if let result { continuation.resume(returning: result) }
    }
  }
}

private final class ReleaseSignal {
  deinit { TestSignal.shared.fire() }
}

nonisolated(unsafe) private var releaseSignalKey: UInt8 = 0

func signalOnRelease(_ object: AnyObject) {
  objc_setAssociatedObject(object, &releaseSignalKey, ReleaseSignal(), .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
}

@discardableResult
func eventually(_ holds: () async -> Bool) async -> Bool {
  await waits(timeout: .seconds(10), holds)
}

@discardableResult
func waits(timeout: Duration = .seconds(10), _ holds: () async -> Bool) async -> Bool {
  TestChange.install { TestSignal.shared.fire() }
  let parts = timeout.components
  let seconds = Double(parts.seconds) + Double(parts.attoseconds) / 1e18
  let deadline = DispatchTime.now() + seconds
  while true {
    let generation = TestSignal.shared.current
    if await holds() { return true }
    if DispatchTime.now() >= deadline { return false }
    if !(await TestSignal.shared.changed(since: generation, deadline: deadline)) { return false }
  }
}

func eventually(_ condition: String, timeout: Duration = .seconds(5), fileID: String = #fileID, filePath: String = #filePath, line: Int = #line, column: Int = #column, _ holds: () async -> Bool) async {
  if !(await waits(timeout: timeout, holds)) {
    Issue.record("never became true within \(timeout): \(condition)", sourceLocation: SourceLocation(fileID: fileID, filePath: filePath, line: line, column: column))
  }
}
