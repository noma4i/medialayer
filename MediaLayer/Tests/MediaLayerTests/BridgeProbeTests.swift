import Testing
@testable import MediaLayer

struct BridgeProbeTests {
  @Test
  func versionComesFromRust() {
    #expect(version() == "2.0.0")
  }
}
