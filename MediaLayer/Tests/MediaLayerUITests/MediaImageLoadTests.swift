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
    let origin = TestFetcher()
    origin.serve(payload, at: address)
    let gate = HeldUntilOpened()
    var ticks = 0
    let heartbeat = Task { @MainActor in
      while !Task.isCancelled {
        ticks += 1
        if ticks == 20 { await gate.open() }
        try? await Task.sleep(for: .milliseconds(1))
      }
    }

    _ = try await MediaImageLoad.load(url: URL(string: address)!, directory: try directory(), fetcher: HeldFetcher(origin: origin, gate: gate))
    heartbeat.cancel()

    #expect(ticks >= 20, "ticks=\(ticks)")
  }
}

private actor HeldUntilOpened {
  private var isOpen = false
  private var waiting: [CheckedContinuation<Void, Never>] = []

  func open() {
    isOpen = true
    waiting.forEach { $0.resume() }
    waiting.removeAll()
  }

  func wait() async {
    if isOpen { return }
    await withCheckedContinuation { waiting.append($0) }
  }
}

private final class HeldFetcher: Fetcher, @unchecked Sendable {
  let origin: TestFetcher
  let gate: HeldUntilOpened

  init(origin: TestFetcher, gate: HeldUntilOpened) {
    self.origin = origin
    self.gate = gate
  }

  func fetch(request: FetchRequest) async throws -> FetchResponse {
    await gate.wait()
    return try await origin.fetch(request: request)
  }
}
