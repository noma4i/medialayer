import Foundation

struct DeliveryToken: Sendable {
  fileprivate let generation: UInt64
}

/// Open(generation) or Closed. A delivery runs on Main only while its token is the current open generation.
final class DeliveryGate: @unchecked Sendable {
  private let lock = NSLock()
  private var generation: UInt64 = 0
  private var isOpen = false

  func open() -> DeliveryToken {
    lock.withLock {
      generation &+= 1
      isOpen = true
      return DeliveryToken(generation: generation)
    }
  }

  func close() {
    lock.withLock {
      isOpen = false
      generation &+= 1
    }
  }

  func deliver(_ token: DeliveryToken, _ body: @escaping @MainActor @Sendable () -> Void) {
    Task { @MainActor in
      guard accepts(token) else { return }
      body()
    }
  }

  private func accepts(_ token: DeliveryToken) -> Bool {
    lock.withLock { isOpen && generation == token.generation }
  }
}
