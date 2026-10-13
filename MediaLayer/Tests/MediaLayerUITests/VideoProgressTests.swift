import Testing
@testable import MediaLayerUI

struct VideoProgressTests {
  @Test(arguments: [
    (UInt64(0), UInt64?(8000), 0.0 as Double?),
    (2000, 8000, 0.25),
    (4000, 8000, 0.5),
    (8000, 8000, 1.0),
    (9000, 8000, 1.0),
    (3000, nil, nil),
    (3000, 0, nil)
  ])
  func positionOverDurationIsTheFractionAndAMissingDurationHasNone(position: UInt64, duration: UInt64?, fraction: Double?) {
    #expect(VideoProgress.fraction(positionMs: position, durationMs: duration) == fraction)
  }
}
