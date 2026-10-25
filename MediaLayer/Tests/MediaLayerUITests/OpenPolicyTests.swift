import Foundation
import MediaLayer
import MediaLayerTesting
import Testing
@testable import MediaLayerUI

private final class ConfigBox: @unchecked Sendable {
  private let lock = NSLock()
  private var stored: OpenConfig?

  var config: OpenConfig? { lock.withLock { stored } }
  func keep(_ config: OpenConfig) { lock.withLock { stored = config } }
}

extension GlobalMedia {
  struct OpenPolicyTests {
    @Test
    func inv_ffi_19_a_layer_opened_without_a_flag_is_silent_on_a_simulator_and_not_on_a_device() async throws {
      var silent: [Bool] = []
      for simulator in [nil, false] as [Bool?] {
        let box = ConfigBox()
        let settings = UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!)
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let build: @Sendable (OpenConfig) throws -> MediaLayer = { box.keep($0); return try MediaLayer.open(config: $0) }
        let defaults: @Sendable () throws -> SpecDefaults = { SpecDefaults(defaults: defaultLimits(), pool: defaultPoolSettings(), namespaceHeaders: []) }
        let media: MediaLayer
        if let simulator {
          media = try await MediaLayer.open(
            defaults: defaults, fetcher: TestFetcher(), settings: settings, directory: directory,
            audioFocus: .mix, imageMemoryBytes: 1_000_000, onSimulator: simulator, build: build
          )
        } else {
          media = try await MediaLayer.open(
            defaults: defaults, fetcher: TestFetcher(), settings: settings, directory: directory,
            audioFocus: .mix, imageMemoryBytes: 1_000_000, build: build
          )
        }
        silent.append(try #require(box.config).silent)
        media.close()
      }

      #expect(silent == [true, false])
    }

    @Test
    func theForwardHeadersAndTheEvictionOverridesOfTheSpecDefaultsReachTheOpenConfig() async throws {
      let box = ConfigBox()
      let resolved = SpecDefaults(
        defaults: defaultLimits(), pool: defaultPoolSettings(), namespaceHeaders: [],
        forwardHeaders: ["X-Token"], evictVideo: .largestFirst, evictImage: .fifo, evictAudio: .lfu
      )

      _ = try await MediaLayer.open(
        defaults: { resolved }, fetcher: TestFetcher(),
        settings: UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!),
        directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString),
        audioFocus: .mix, imageMemoryBytes: 1_000_000,
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
