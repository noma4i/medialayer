import Foundation

final class DemandBox: @unchecked Sendable {
  private let lock = NSLock()
  private var demand = AudioDemand()
  private let focus: MediaAudioFocus

  init(focus: MediaAudioFocus) {
    self.focus = focus
  }

  func update(_ change: (inout AudioDemand) -> Bool) -> Bool {
    lock.withLock { change(&demand) }
  }

  func target() -> SessionTarget {
    lock.withLock { demand.target(for: focus) }
  }
}
