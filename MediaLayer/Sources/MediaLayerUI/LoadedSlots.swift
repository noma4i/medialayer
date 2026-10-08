import Foundation
import QuartzCore

public final class LoadedSlots: @unchecked Sendable {
  public static let shared = LoadedSlots()

  private let lock = NSLock()
  private var slots: [String: WeakSlot] = [:]

  public init() {}

  public func slot(for key: String) -> AVQueuePlayerSlot? {
    lock.withLock { slots[key]?.value }
  }

  func assign(_ slot: AVQueuePlayerSlot, key: String) {
    lock.withLock { slots[key] = WeakSlot(slot) }
  }

  func release(key: String, slot: AVQueuePlayerSlot) {
    lock.withLock {
      guard slots[key]?.value === slot || slots[key]?.value == nil else { return }
      slots[key] = nil
    }
  }
}

private struct WeakSlot {
  weak var value: AVQueuePlayerSlot?

  init(_ value: AVQueuePlayerSlot) { self.value = value }
}

public final class HostLayers: @unchecked Sendable {
  public static let shared = HostLayers()

  private struct WeakLayer {
    weak var value: CALayer?
  }

  private let lock = NSLock()
  private var layers: [UInt64: WeakLayer] = [:]
  private var next: UInt64 = 1

  public init() {}

  public func register(_ layer: CALayer) -> UInt64 {
    lock.withLock {
      let token = next
      next += 1
      layers[token] = WeakLayer(value: layer)
      return token
    }
  }

  public func unregister(_ token: UInt64) {
    lock.withLock { layers[token] = nil }
  }

  func layer(for token: UInt64) -> CALayer? {
    lock.withLock { layers[token]?.value }
  }
}
