import Foundation
import Testing
@testable import MediaLayer
import MediaLayerTesting

private final class ReloadSlot: PlayerSlot, PlayerSlotFactory, @unchecked Sendable {
  let plays = AsyncStream<Bool>.makeStream()
  private let lock = NSLock()
  private var attached = false
  private var loaded: UInt64 = 0

  var generation: UInt64 { lock.withLock { loaded } }

  func create(slot: UInt32) async -> CreatedPlayer { CreatedPlayer(player: self) }
  func apply(command: SlotCall) async {
    switch command {
    case let .load(_, _, generation): lock.withLock { loaded = generation }
    case .unload, .detach: lock.withLock { attached = false }
    case .play: plays.continuation.yield(lock.withLock { attached })
    case let .attach(viewToken): lock.withLock { attached = viewToken == 42 }
    default: break
    }
  }
  func dispose() async {}
  func sample() -> SlotSample {
    SlotSample(positionMs: 0, durationMs: nil, isLooping: false, viewAttached: lock.withLock { attached }, frameRendered: false)
  }
}

private final class ReloadSettings: SettingsStore, @unchecked Sendable {
  func read(key: String) -> String? { nil }
  func write(key: String, value: String) {}
  func remove(key: String) {}
}

@Suite(.timeLimit(.minutes(1)))
struct ReloadTests {
  @Test
  func inv_api_06_reload_restores_the_attached_view_before_playing() async throws {
    let slot = ReloadSlot()
    let media = try MediaLayer.open(config: OpenConfig(
      dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
      fetcher: TestFetcher(), settings: ReloadSettings(), slots: slot,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(), pool: defaultPoolSettings()
    ))
    let key = "file:///clip.mp4"
    let events = media.playerEvents()
    media.attachView(key: key, token: 42)
    media.replaceWindow(active: 0, sources: [MediaItem(kind: .video, url: key, headers: [:], cover: nil, blurHash: nil, strategy: .window)])
    media.setPlaying(key: key, playing: true)
    var plays = slot.plays.stream.makeAsyncIterator()
    #expect(await plays.next() == true)
    events.failed(slot: 0, generation: slot.generation, status: 0, detail: "decoder failed")
    #expect(await plays.next() == true)
  }
}
