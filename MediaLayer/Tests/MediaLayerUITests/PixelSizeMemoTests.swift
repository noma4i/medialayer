import Foundation
import MediaLayer
import Testing
@testable import MediaLayerUI

private final class LockedCounter: @unchecked Sendable {
  private let lock = NSLock()
  private var count = 0
  var value: Int { lock.withLock { count } }
  func increment() { lock.withLock { count += 1 } }
}

struct PixelSizeMemoTests {
  @Test
  func inv_image_40_tilesOfOneSizeCostOneImagePixelSizeCallAndTheMapStaysBounded() {
    let calls = LockedCounter()
    let memo = PixelSizeMemo { width, _, _, _, _ in calls.increment(); return PixelSize(width: UInt32(width), height: 1) }

    for _ in 0 ..< 30 { #expect(memo.size(width: 100, height: 100, scale: 2, declared: nil, crop: true) == PixelSize(width: 100, height: 1)) }
    #expect(calls.value == 1)
    for width in 1 ... 70 { _ = memo.size(width: Double(width) + 1000, height: 1, scale: 1, declared: nil, crop: true) }
    let before = calls.value
    _ = memo.size(width: 100, height: 100, scale: 2, declared: nil, crop: true)
    #expect(calls.value == before + 1)
  }
}
