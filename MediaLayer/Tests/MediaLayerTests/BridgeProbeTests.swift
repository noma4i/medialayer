import Testing
@testable import MediaLayer

struct BridgeProbeTests {
  @Test
  func versionComesFromRust() {
    #expect(version() == "3.4.0")
  }
}
