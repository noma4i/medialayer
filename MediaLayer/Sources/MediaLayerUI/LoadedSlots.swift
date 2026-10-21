import Foundation
import QuartzCore

public final class LoadedSlots: @unchecked Sendable {
  public static let shared = LoadedSlots()

  private let lock = NSLock()
  private var slots: [String: WeakSlot] = [:]
  @MainActor lazy var progress = PlaybackProgress(slots: self)

  public init() {}

  public func slot(for key: String) -> AVQueuePlayerSlot? {
    lock.withLock { slots[key]?.value }
  }

  func assign(_ slot: AVQueuePlayerSlot, key: String) {
    lock.withLock { slots[key] = WeakSlot(slot) }
    Task { @MainActor in progress.changed(key) }
  }

  func release(key: String, slot: AVQueuePlayerSlot) {
    lock.withLock {
      guard slots[key]?.value === slot || slots[key]?.value == nil else { return }
      slots[key] = nil
    }
    Task { @MainActor in progress.changed(key) }
  }
}

private struct WeakSlot {
  weak var value: AVQueuePlayerSlot?

  init(_ value: AVQueuePlayerSlot) { self.value = value }
}

struct HostClaim {
  let token: UInt64
  let owner: UUID
}

enum HostRelease: Equatable {
  case none
  case detach
  case handover(UInt64)
}

public final class HostLayers: @unchecked Sendable {
  public static let shared = HostLayers()

  private struct WeakLayer {
    weak var value: CALayer?
  }

  private let lock = NSLock()
  private var layers: [UInt64: WeakLayer] = [:]
  private var claims: [String: [HostClaim]] = [:]
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

  func claim(key: String, token: UInt64, owner: UUID, isPlaying: Bool) -> Bool {
    lock.withLock {
      var stack = claims[key] ?? []
      stack.removeAll { $0.token == token || layers[$0.token]?.value == nil }
      let isFirst = stack.isEmpty
      stack.insert(HostClaim(token: token, owner: owner), at: isPlaying ? stack.count : 0)
      claims[key] = stack
      return isFirst || isPlaying
    }
  }

  func prefer(key: String, owner: UUID) -> UInt64? {
    lock.withLock {
      var stack = claims[key] ?? []
      guard let index = stack.firstIndex(where: { $0.owner == owner }), index != stack.count - 1 else { return nil }
      let claim = stack.remove(at: index)
      stack.append(claim)
      claims[key] = stack
      return claim.token
    }
  }

  func release(key: String, token: UInt64) -> HostRelease {
    lock.withLock {
      var stack = claims[key] ?? []
      let wasOwner = stack.last?.token == token
      stack.removeAll { $0.token == token }
      stack.removeAll { layers[$0.token]?.value == nil }
      claims[key] = stack.isEmpty ? nil : stack
      guard wasOwner else { return .none }
      return stack.last.map { HostRelease.handover($0.token) } ?? .detach
    }
  }

  func layer(for token: UInt64) -> CALayer? {
    lock.withLock { layers[token]?.value }
  }
}
