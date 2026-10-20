import Foundation
import Testing
@testable import MediaLayerUI

struct KeyClaimsLifetimeTests {
  @Test
  func inv_native_01_claims_do_not_retain_their_places() {
    weak var released: RecordingPlaces?
    let claims: KeyClaims = {
      let places = RecordingPlaces()
      released = places
      return KeyClaims.of(places)
    }()

    #expect(released == nil)
    withExtendedLifetime(claims) {}
  }
}
