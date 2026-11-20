import Testing
@testable import MediaLayer

struct BridgeProbeTests {
  @Test
  func versionComesFromRust() {
    #expect(version() == "3.49.0")
  }
}
