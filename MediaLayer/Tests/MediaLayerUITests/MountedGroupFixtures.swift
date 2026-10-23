import Foundation
import MediaLayer

final class QuietSettings: SettingsStore, @unchecked Sendable {
  func read(key: String) -> String? { nil }
  func write(key: String, value: String) {}
  func remove(key: String) {}
}

final class UnusedFetcher: Fetcher, @unchecked Sendable {
  func fetch(request: FetchRequest) async throws -> FetchResponse { throw MediaError.Fetch(detail: "unused origin") }
}
