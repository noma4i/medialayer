import Foundation
import Testing
@testable import MediaLayerUI

struct UserDefaultsSettingsTests {
  @Test
  func writtenValueIsReadAndRemovedValueIsGone() throws {
    let suite = "medialayer.tests.\(UUID().uuidString)"
    let defaults = try #require(UserDefaults(suiteName: suite))
    defer { defaults.removePersistentDomain(forName: suite) }
    let settings = UserDefaultsSettings(defaults: defaults)

    settings.write(key: "medialayer.limits", value: "{}")
    #expect(settings.read(key: "medialayer.limits") == "{}")
    settings.remove(key: "medialayer.limits")

    #expect(settings.read(key: "medialayer.limits") == nil)
  }
}
