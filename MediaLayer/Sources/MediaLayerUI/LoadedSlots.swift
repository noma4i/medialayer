import Foundation
import QuartzCore
import ObjectiveC

nonisolated(unsafe) private var slotsKey: UInt8 = 0
nonisolated(unsafe) private var hostsKey: UInt8 = 0

public final class LoadedSlots: @unchecked Sendable {
  public static func of(_ places: MediaPlaces) -> LoadedSlots {
    objc_sync_enter(places)
    defer { objc_sync_exit(places) }
    if let existing = objc_getAssociatedObject(places, &slotsKey) as? LoadedSlots { return existing }
    let slots = LoadedSlots()
    install(slots, on: places)
    return slots
  }

  static func install(_ slots: LoadedSlots, on places: MediaPlaces) {
    objc_setAssociatedObject(places, &slotsKey, slots, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
  }

  private let lock = NSLock()
  private var slots: [String: WeakSlot] = [:]
  @MainActor lazy var progress = PlaybackProgress(slots: self)

  public init() {}

  public func slot(for key: String) -> AVQueuePlayerSlot? {
    lock.withLock { slots[key]?.value }
  }

  public func snapshot(for key: String) -> PlayerSnapshot? { slot(for: key)?.snapshot() }

  func assign(_ slot: AVQueuePlayerSlot, key: String) {
    lock.withLock { slots[key] = WeakSlot(slot) }
    TestChange.fire()
    Task { @MainActor in progress.changed(key) }
  }

  func release(key: String, slot: AVQueuePlayerSlot) {
    lock.withLock {
      guard slots[key]?.value === slot || slots[key]?.value == nil else { return }
      slots[key] = nil
    }
    TestChange.fire()
    Task { @MainActor in progress.changed(key) }
  }
}

private struct WeakSlot {
  weak var value: AVQueuePlayerSlot?

  init(_ value: AVQueuePlayerSlot) { self.value = value }
}

public final class HostLayers: @unchecked Sendable {
  public static func of(_ places: MediaPlaces) -> HostLayers {
    objc_sync_enter(places)
    defer { objc_sync_exit(places) }
    if let existing = objc_getAssociatedObject(places, &hostsKey) as? HostLayers { return existing }
    let hosts = HostLayers()
    install(hosts, on: places)
    return hosts
  }

  static func install(_ hosts: HostLayers, on places: MediaPlaces) {
    objc_setAssociatedObject(places, &hostsKey, hosts, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
  }

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
