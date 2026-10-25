import Foundation
import MediaLayer
import MediaLayerTesting
import Testing
@testable import MediaLayerUI

@Suite(.serialized, .timeLimit(.minutes(1)))
struct MediaImageLoadTests {
  private let address = "https://img.example/avatar.png"
  private let payload = Data(repeating: 7, count: 300)

  private func directory() throws -> URL {
    let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
    try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
    return url
  }

  @Test
  func inv_image_56_second_load_of_a_url_comes_from_the_cache_without_network() async throws {
    let fetcher = TestFetcher()
    fetcher.serve(payload, at: address)
    let directory = try directory()

    let first = try await MediaImageLoad.load(url: URL(string: address)!, directory: directory, fetcher: fetcher)
    let second = try await MediaImageLoad.load(url: URL(string: address)!, directory: directory, fetcher: fetcher)

    #expect(first.source == .network)
    #expect(second.source == .cache)
    #expect(second.bytes == payload)
    #expect(fetcher.requestCount(for: address) == 1)
  }

  @Test
  func inv_image_58_slow_origin_ends_with_a_timeout_error() async throws {
    let fetcher = TestFetcher()
    fetcher.serve(payload, at: address)
    fetcher.delay(.seconds(30), for: address)

    await #expect(throws: MediaError.self) {
      try await MediaImageLoad.load(url: URL(string: address)!, directory: try directory(), timeout: 0.2, fetcher: fetcher)
    }
  }

  @MainActor
  @Test
  func inv_ios_03_load_does_not_block_the_main_thread() async throws {
    let fetcher = TestFetcher()
    fetcher.serve(payload, at: address)
    fetcher.delay(.milliseconds(400), for: address)
    let clock = ContinuousClock()
    var last = clock.now
    var longestStall = Duration.zero
    var ticks = 0
    let heartbeat = Task { @MainActor in
      while !Task.isCancelled {
        let now = clock.now
        longestStall = max(longestStall, now - last)
        last = now
        ticks += 1
        try? await Task.sleep(for: .milliseconds(10))
      }
    }

    _ = try await MediaImageLoad.load(url: URL(string: address)!, directory: try directory(), fetcher: fetcher)
    heartbeat.cancel()

    #expect(ticks >= 2, "ticks=\(ticks)")
    #expect(longestStall < .milliseconds(200), "longestStall=\(longestStall)")
  }
}
