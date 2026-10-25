import Foundation
import Testing
@testable import MediaLayerUI

private final class Ledger: @unchecked Sendable {
  private let lock = NSLock()
  private(set) var applied: [Int] = []
  private(set) var onMain = false

  func record(_ value: Int) {
    lock.withLock {
      applied.append(value)
      if Thread.isMainThread { onMain = true }
    }
  }

  var count: Int { lock.withLock { applied.count } }
}

@Suite struct LatestDrainTests {
  @Test func heldApplyKeepsOneRunningAndOneMergedPendingValue() async {
    let ledger = Ledger()
    let entered = DispatchSemaphore(value: 0)
    let release = DispatchSemaphore(value: 0)
    let drain = LatestDrain<Int>(merge: { _, next in next }) { value in
      ledger.record(value)
      if value == 0 {
        entered.signal()
        release.wait()
      }
    }

    drain.submit(0)
    entered.wait()
    for value in 1...10_000 { drain.submit(value) }
    release.signal()

    await eventually("the latest value is applied") { ledger.applied.last == 10_000 }
    #expect(ledger.applied == [0, 10_000])
  }

  @Test func applyNeverRunsOnTheMainThread() async {
    let ledger = Ledger()
    let drain = LatestDrain<Int>(merge: { _, next in next }, apply: ledger.record)

    await MainActor.run { drain.submit(1) }

    await eventually("applied") { ledger.count == 1 }
    #expect(!ledger.onMain)
  }

  @Test func aHeldApplyOfOneDrainDoesNotDelayAnotherDrain() async {
    let entered = DispatchSemaphore(value: 0)
    let release = DispatchSemaphore(value: 0)
    let held = LatestDrain<Int>(merge: { _, next in next }) { _ in
      entered.signal()
      release.wait()
    }
    let ledger = Ledger()
    let other = LatestDrain<Int>(merge: { _, next in next }, apply: ledger.record)

    held.submit(0)
    entered.wait()
    other.submit(1)

    await eventually("the other drain applies while the first is held") { ledger.count == 1 }
    release.signal()
  }

  @Test func aReplacedSnapshotIsReleasedInsideTheDrainNotOnTheSubmittingThread() async {
    let log = ReleaseLog()
    let entered = DispatchSemaphore(value: 0)
    let release = DispatchSemaphore(value: 0)
    let drain = LatestDrain<Snapshot>(merge: { _, next in next }) { snapshot in
      if snapshot.id == 0 {
        entered.signal()
        release.wait()
      }
    }

    drain.submit(Snapshot(id: 0, probe: Releasing(log)))
    entered.wait()
    await MainActor.run {
      drain.submit(Snapshot(id: 1, probe: Releasing(log)))
      drain.submit(Snapshot(id: 2, probe: Releasing(log)))
    }
    release.signal()

    await eventually("every snapshot is released") { log.count == 3 }
    #expect(log.releasedOnMain == 0)
  }
}

private final class ReleaseLog: @unchecked Sendable {
  private let lock = NSLock()
  private var total = 0
  private var main = 0
  var count: Int { lock.withLock { total } }
  var releasedOnMain: Int { lock.withLock { main } }
  func released() {
    lock.withLock {
      total += 1
      if Thread.isMainThread { main += 1 }
    }
  }
}

private final class Releasing: @unchecked Sendable {
  private let log: ReleaseLog
  init(_ log: ReleaseLog) { self.log = log }
  deinit { log.released() }
}

private struct Snapshot: Sendable {
  let id: Int
  let probe: Releasing
}
