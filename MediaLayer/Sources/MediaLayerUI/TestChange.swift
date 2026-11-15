import Foundation

enum TestChange {
  private static let lock = NSLock()
  nonisolated(unsafe) private static var callback: (@Sendable () -> Void)?

  static func install(_ callback: @escaping @Sendable () -> Void) {
    lock.withLock { self.callback = callback }
  }

  static func fire() {
    lock.withLock { callback }?()
  }
}
