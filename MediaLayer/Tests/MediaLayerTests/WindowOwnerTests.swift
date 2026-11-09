import Foundation
import Testing
@testable import MediaLayer
import MediaLayerTesting

private final class OwnerSlot: PlayerSlot, PlayerSlotFactory, @unchecked Sendable {
  let calls = AsyncStream<String>.makeStream()

  func create(slot: UInt32) async -> CreatedPlayer { CreatedPlayer(player: self) }
  func apply(command: SlotCall) async {
    switch command {
    case let .load(key, _, _): calls.continuation.yield("load \(key)")
    case .unload: calls.continuation.yield("unload")
    default: break
    }
  }
  func dispose() async {}
  func sample() -> SlotSample {
    SlotSample(positionMs: 0, durationMs: nil, isLooping: false, viewAttached: false, frameRendered: false)
  }
}

private final class OwnerSettings: SettingsStore, @unchecked Sendable {
  func read(key: String) -> String? { nil }
  func write(key: String, value: String) {}
  func remove(key: String) {}
}

@Suite(.timeLimit(.minutes(1)))
struct WindowOwnerTests {
  private func item(_ url: String) -> MediaItem {
    MediaItem(kind: .video, url: url, headers: [:], cover: nil, blurHash: nil, strategy: .window)
  }

  @Test
  func inv_ingress_49_a_new_window_without_sources_leaves_the_stage_to_the_window_that_has_them() async throws {
    let slot = OwnerSlot()
    let media = try MediaLayer.open(config: OpenConfig(
      dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
      availableBytes: UInt64.max, fetcher: TestFetcher(), settings: OwnerSettings(), slots: slot,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(), pool: defaultPoolSettings()
    ))
    let feed = try media.openVideoWindow()
    try feed.replace(active: 0, sources: [item("file:///feed.mp4")])
    var seen: [String] = []
    var calls = slot.calls.stream.makeAsyncIterator()
    while !seen.contains("load file:///feed.mp4"), let call = await calls.next() { seen.append(call) }

    let offscreen = try media.openVideoWindow()
    try offscreen.replace(active: 0, sources: [])
    try offscreen.moveTo(active: 0)
    try offscreen.setActive(isActive: false)
    try feed.replace(active: 0, sources: [item("file:///feed.mp4"), item("file:///next.mp4")])
    while !seen.contains("load file:///next.mp4"), let call = await calls.next() { seen.append(call) }

    #expect(seen.filter { $0.hasPrefix("load") || $0 == "unload" } == ["load file:///feed.mp4", "load file:///next.mp4"])
  }
}
