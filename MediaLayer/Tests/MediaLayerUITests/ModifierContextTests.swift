#if canImport(UIKit)
import Foundation
import MediaLayer
import MediaLayerTesting
import Observation
import SwiftUI
import Testing
import UIKit
@testable import MediaLayerUI

private final class IdleSlot: PlayerSlot, PlayerSlotFactory, @unchecked Sendable {
  func create(slot: UInt32) async -> CreatedPlayer { CreatedPlayer(player: self) }
  func apply(command: SlotCall) async {}
  func dispose() async {}
  func sample() -> SlotSample { SlotSample(positionMs: 0, durationMs: nil, isLooping: false, viewAttached: false, frameRendered: false) }
}

private final class StatsCountingPlaces: MediaPlaces, @unchecked Sendable {
  private let inner: MediaLayer
  private let lock = NSLock()
  private var statsCalls = 0
  var imageStatsCalls: Int { lock.withLock { statsCalls } }

  init(_ inner: MediaLayer) { self.inner = inner }

  func imageStats() -> ImageCounters {
    lock.withLock { statsCalls += 1 }
    return inner.imageStats()
  }
  func cacheKey(url: String, headers: [String: String]) -> String { inner.cacheKey(url: url, headers: headers) }
  func fetchImage(url: String, headers: [String: String]) async throws -> ImageFetchResult { try await inner.fetchImage(url: url, headers: headers) }
  func placeholder(source: MediaItem, ready: Bool) async -> Placeholder { await inner.placeholder(source: source, ready: ready) }
  func tonePixelsNeeded(source: MediaItem) async -> Bool { await inner.tonePixelsNeeded(source: source) }
  func playUrl(source: MediaItem) async throws -> MediaPlayer { try await inner.playUrl(source: source) }
  func readRoute(item: MediaItem) async throws -> PlaybackRoute { try await inner.readRoute(item: item) }
  func stateStream(key: String) -> AsyncStream<MediaState> { inner.stateStream(key: key) }
  func retry(item: MediaItem) async throws { try await inner.retry(item: item) }
  func toneStream(source: MediaItem, band: Float) -> AsyncStream<ToneUpdate> { inner.toneStream(source: source, band: band) }
  func mountPlace(item: MediaItem) throws -> any PlaceHandle { try inner.mountPlace(item: item) }
  func mountVideoWindow() throws -> any WindowHandle { try inner.mountVideoWindow() }
  func mountImageWindow(name: String, rowsPerPage: Int, reversed: Bool) throws -> any WindowHandle {
    try inner.mountImageWindow(name: name, rowsPerPage: rowsPerPage, reversed: reversed)
  }
  func seek(key: String, fraction: Double) throws { try inner.seek(key: key, fraction: fraction) }
  func setMutedAll(muted: Bool) throws { try inner.setMutedAll(muted: muted) }
  func setTonePreparation(preparation: TonePreparation) throws { try inner.setTonePreparation(preparation: preparation) }
}

@Observable
@MainActor
private final class ParentModel {
  var rebuilds = 0
  var bodies = 0
}

private struct Parent: View {
  let model: ParentModel
  let places: MediaPlaces

  var body: some View {
    let _ = model.rebuilds
    let _ = { model.bodies += 1 }()
    Color.clear.mediaLayer(places).frame(width: 40, height: 40)
  }
}

extension GlobalMedia {
  @MainActor
  @Suite(.serialized, .timeLimit(.minutes(2)))
  struct ModifierContextTests {
    @Test
    func inv_native_31_rebuildingTheModifierKeepsOneContext() async throws {
      let media = try MediaLayer.open(config: OpenConfig(
        dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
        fetcher: TestFetcher(), settings: QuietSettings(), slots: IdleSlot(),
        device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
        defaults: defaultLimits(), pool: defaultPoolSettings(), audioFocus: .exclusive
      ))
      let places = StatsCountingPlaces(media)
      let model = ParentModel()
      let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 100, height: 100))
      window.rootViewController = UIHostingController(rootView: Parent(model: model, places: places))
      window.isHidden = false
      defer {
        window.isHidden = true
        window.rootViewController = nil
      }
      for _ in 0..<400 where model.bodies == 0 { try await Task.sleep(for: .milliseconds(25)) }
      let first = model.bodies
      #expect(first > 0)

      for _ in 0..<10 {
        let seen = model.bodies
        model.rebuilds += 1
        for _ in 0..<400 where model.bodies == seen { try await Task.sleep(for: .milliseconds(25)) }
      }

      #expect(model.bodies >= first + 10)
      #expect(places.imageStatsCalls == 1)
    }
  }
}
#endif
