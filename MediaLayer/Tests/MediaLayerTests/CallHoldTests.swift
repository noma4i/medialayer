import Foundation
import Testing
@testable import MediaLayer
import MediaLayerTesting

private final class CallSlot: PlayerSlot, PlayerSlotFactory, @unchecked Sendable {
  let log = AsyncStream<String>.makeStream()
  private let lock = NSLock()
  private var count = 0

  var plays: Int { lock.withLock { count } }

  func create(slot: UInt32) async -> CreatedPlayer { CreatedPlayer(player: self) }
  func apply(command: SlotCall) async {
    switch command {
    case .play:
      lock.withLock { count += 1 }
      log.continuation.yield("play")
    case .pause: log.continuation.yield("pause")
    default: break
    }
  }
  func dispose() async {}
  func sample() -> SlotSample {
    SlotSample(positionMs: 0, durationMs: nil, isLooping: false, viewAttached: false, frameRendered: false)
  }
}

private final class CallSettings: SettingsStore, @unchecked Sendable {
  func read(key: String) -> String? { nil }
  func write(key: String, value: String) {}
  func remove(key: String) {}
}

private let clip = "file:///clip.mp4"
private let item = MediaItem(kind: .video, url: clip, headers: [:], cover: nil, blurHash: nil, strategy: .window)

@Suite(.timeLimit(.minutes(1)))
struct CallHoldTests {
  @Test
  func inv_call_12_a_call_hold_pauses_the_playback_and_its_release_does_not_resume_it() async throws {
    let slot = CallSlot()
    let media = try MediaLayer.open(config: OpenConfig(
      dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
      availableBytes: UInt64.max, fetcher: TestFetcher(), settings: CallSettings(), slots: slot,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(), pool: defaultPoolSettings()
    ))
    media.replaceWindow(active: 0, sources: [item])
    media.setPlaying(key: clip, playing: true, groups: [nil])
    var events = slot.log.stream.makeAsyncIterator()
    #expect(await events.next() == "play")

    let hold = try media.holdCall()
    #expect(await events.next() == "pause")
    await #expect(throws: MediaError.self) { _ = try await media.playUrl(source: item) }

    hold.release()
    _ = try await media.playUrl(source: item)
    #expect(slot.plays == 1, "the release resumed the playback")
    media.setPlaying(key: clip, playing: true, groups: [nil])
    #expect(await events.next() == "play")
    #expect(slot.plays == 2)
  }
}
