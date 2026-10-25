import Foundation
import MediaLayer
import Testing
@testable import MediaLayerUI

private final class ConfigBox: @unchecked Sendable {
  private let lock = NSLock()
  private var stored: OpenConfig?

  var config: OpenConfig? { lock.withLock { stored } }
  func keep(_ config: OpenConfig) { lock.withLock { stored = config } }
}

private final class OfflineOrigin: Fetcher, @unchecked Sendable {
  func fetch(request: FetchRequest) async throws -> FetchResponse { throw MediaError.Fetch(detail: "offline") }
}

extension GlobalMedia {
  struct OpenPolicyTests {
    @Test
    func theForwardHeadersAndTheEvictionOverridesOfTheSpecDefaultsReachTheOpenConfig() async throws {
      let box = ConfigBox()
      let resolved = SpecDefaults(
        defaults: defaultLimits(), pool: defaultPoolSettings(), namespaceHeaders: [],
        forwardHeaders: ["X-Token"], evictVideo: .largestFirst, evictImage: .fifo, evictAudio: .lfu
      )

      _ = try await MediaLayer.open(
        defaults: { resolved }, fetcher: OfflineOrigin(),
        settings: UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!),
        directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString),
        silent: false, audioFocus: .mix, imageMemoryBytes: 1_000_000,
        build: { box.keep($0); return try MediaLayer.open(config: $0) }
      )

      let config = try #require(box.config)
      #expect(config.forwardHeaders == ["X-Token"])
      #expect(config.evictVideo == .largestFirst)
      #expect(config.evictImage == .fifo)
      #expect(config.evictAudio == .lfu)
    }
  }
}
