import Foundation
import Testing

struct WaitsTickTests {
  @Test
  func a_wait_sees_a_state_that_changes_without_a_fire() async {
    let flag = Flag()
    DispatchQueue.global().asyncAfter(deadline: .now() + .milliseconds(100)) { flag.set() }

    let reached = await waits(timeout: .seconds(2)) { flag.value }

    #expect(reached)
  }
}

private final class Flag: @unchecked Sendable {
  private let lock = NSLock()
  private var on = false
  var value: Bool { lock.withLock { on } }
  func set() { lock.withLock { on = true } }
}
