import Foundation
import MediaLayer
import ObjectiveC

nonisolated(unsafe) private var pixelSizesKey: UInt8 = 0

enum FfiQueue {
  private static let queue = DispatchQueue(label: "com.noma4i.medialayer.ffi")

  static func post(_ call: @escaping @Sendable () -> Void) { queue.async(execute: call) }

  static func run<Result: Sendable>(_ call: @escaping @Sendable () -> Result) async -> Result {
    await withCheckedContinuation { continuation in queue.async { continuation.resume(returning: call()) } }
  }
}

extension ImageCounters {
  func recordLater(work: ImageWork) { FfiQueue.post { self.record(work: work) } }
}

final class PixelSizeMemo: @unchecked Sendable {
  private struct Args: Hashable {
    let width: Double
    let height: Double
    let scale: Double
    let declaredWidth: UInt32?
    let declaredHeight: UInt32?
    let crop: Bool
  }

  private static let capacity = 64
  private let compute: @Sendable (Double, Double, Double, PixelSize?, Bool) -> PixelSize?
  private let lock = NSLock()
  private var sizes: [Args: PixelSize?] = [:]
  private var order: [Args] = []

  init(compute: @escaping @Sendable (Double, Double, Double, PixelSize?, Bool) -> PixelSize? = { imagePixelSize(width: $0, height: $1, scale: $2, declared: $3, crop: $4) }) {
    self.compute = compute
  }

  static func of(_ places: MediaPlaces) -> PixelSizeMemo {
    objc_sync_enter(places)
    defer { objc_sync_exit(places) }
    if let existing = objc_getAssociatedObject(places, &pixelSizesKey) as? PixelSizeMemo { return existing }
    let created = PixelSizeMemo()
    objc_setAssociatedObject(places, &pixelSizesKey, created, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
    return created
  }

  func size(width: Double, height: Double, scale: Double, declared: PixelSize?, crop: Bool) -> PixelSize? {
    let args = Args(width: width, height: height, scale: scale, declaredWidth: declared?.width, declaredHeight: declared?.height, crop: crop)
    if let hit = lock.withLock({ sizes[args] }) { return hit }
    let size = compute(width, height, scale, declared, crop)
    lock.withLock {
      if sizes.updateValue(size, forKey: args) == nil {
        order.append(args)
        if order.count > Self.capacity { sizes[order.removeFirst()] = nil }
      }
    }
    return size
  }
}
