import Foundation
import MediaLayer
import ObjectiveC

nonisolated(unsafe) private var recorderKey: UInt8 = 0

/// Coalesces image work records into totals; one drain applies them, so records cost no task each.
final class ImageRecorder: @unchecked Sendable {
  private let lock = NSLock()
  private var totals = ImageTotals(requests: 0, placementPixels: 0, decodes: 0, decodedPixels: 0, bitmapBytes: 0, finished: 0, memoryHits: 0, cancellations: 0, diskHits: 0, observed: 0)
  private var drain: LatestDrain<Bool>!

  init(counters: ImageCounters) {
    drain = LatestDrain(merge: { _, _ in true }) { [weak self, weak counters] _ in
      guard let self, let counters else { return }
      let taken = self.lock.withLock { () -> ImageTotals in
        defer { self.totals = ImageRecorder.empty }
        return self.totals
      }
      counters.add(totals: taken)
    }
  }

  private static let empty = ImageTotals(requests: 0, placementPixels: 0, decodes: 0, decodedPixels: 0, bitmapBytes: 0, finished: 0, memoryHits: 0, cancellations: 0, diskHits: 0, observed: 0)

  static func of(_ counters: ImageCounters) -> ImageRecorder {
    objc_sync_enter(counters)
    defer { objc_sync_exit(counters) }
    if let existing = objc_getAssociatedObject(counters, &recorderKey) as? ImageRecorder { return existing }
    let created = ImageRecorder(counters: counters)
    objc_setAssociatedObject(counters, &recorderKey, created, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
    return created
  }

  func record(_ work: ImageWork) {
    lock.withLock {
      switch work {
      case .request(let pixels): totals.requests += 1; totals.placementPixels += pixels
      case .decode(let width, let height, let bytes): totals.decodes += 1; totals.decodedPixels += UInt64(width) * UInt64(height); totals.bitmapBytes += bytes
      case .finish(let hit): totals.finished += 1; totals.memoryHits += hit ? 1 : 0
      case .cancel: totals.cancellations += 1
      case .diskHit: totals.diskHits += 1
      case .observe: totals.observed += 1
      case .unobserve: totals.observed -= 1
      }
    }
    drain.submit(true)
  }
}

extension ImageCounters {
  func recordLater(work: ImageWork) { ImageRecorder.of(self).record(work) }
}
