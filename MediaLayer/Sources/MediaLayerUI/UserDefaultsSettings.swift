import Foundation
import MediaLayer

public final class UserDefaultsSettings: SettingsStore, @unchecked Sendable {
  private let defaults: UserDefaults

  public init(defaults: UserDefaults = .standard) {
    self.defaults = defaults
  }

  public func read(key: String) -> String? { defaults.string(forKey: key) }

  public func write(key: String, value: String) { defaults.set(value, forKey: key) }

  public func remove(key: String) { defaults.removeObject(forKey: key) }
}
