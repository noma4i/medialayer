import Testing
@testable import MediaLayer

struct BridgeProbeTests {
  @Test
  func versionComesFromRust() {
    #expect(version() == "1.0.0")
  }
}
