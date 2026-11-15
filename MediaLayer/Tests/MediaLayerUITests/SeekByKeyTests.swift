import Foundation
import Testing
import MediaLayer
import MediaLayerTesting
@testable import MediaLayerUI

private final class SeekSlot: PlayerSlot, PlayerSlotFactory, @unchecked Sendable {
  let log = AsyncStream<String>.makeStream()

  func create(slot: UInt32) async -> CreatedPlayer { CreatedPlayer(player: self) }
  func apply(command: SlotCall) async {
    switch command {
    case let .load(_, _, generation): log.continuation.yield("load \(generation)")
    case let .seek(positionMs): log.continuation.yield("seek \(positionMs)")
    default: break
    }
    TestSignal.shared.fire()
  }
  func dispose() async {}
  func sample() -> SlotSample {
    SlotSample(positionMs: 0, durationMs: 60_000, isLooping: false, viewAttached: false, frameRendered: false)
  }
}

private final class SeekSettings: SettingsStore, @unchecked Sendable {
  func read(key: String) -> String? { nil }
  func write(key: String, value: String) {}
  func remove(key: String) {}
}

private let clip = "file:///clip.mp4"

private func open(_ slot: SeekSlot) throws -> MediaLayer {
  try MediaLayer.open(config: OpenConfig(
    dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
    fetcher: TestFetcher(), settings: SeekSettings(), slots: slot,
    device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
    defaults: defaultLimits(), pool: defaultPoolSettings()
  ))
}

@Suite(.timeLimit(.minutes(1)))
struct SeekByKeyTests {
  @Test
  func inv_ingress_40_aSeekOfAKeyWithoutAMountedPlaceReachesTheCoreAndAppliesWhenTheKeyLoads() async throws {
    let slot = SeekSlot()
    let media = try open(slot)
    let places: any MediaPlaces = media
    var events = slot.log.stream.makeAsyncIterator()

    try places.seek(key: clip, fraction: 0.5)
    media.replaceWindow(active: 0, sources: [MediaItem(kind: .video, url: clip, headers: [:], cover: nil, blurHash: nil, strategy: .window)])
    media.setPlaying(key: clip, playing: true, groups: [nil])

    let loaded = await events.next()?.replacingOccurrences(of: "load ", with: "")
    media.playerEvents().ready(slot: 0, generation: UInt64(loaded ?? "") ?? 0)
    #expect(await events.next() == "seek 30000")
    withExtendedLifetime(media) {}
  }
}
