import Foundation
import Testing
@testable import MediaLayer

private final class ReloadSlot: PlayerSlot, PlayerSlotFactory, @unchecked Sendable {
  let plays = AsyncStream<Bool>.makeStream()
  private let lock = NSLock()
  private var attached = false

  func create(slot: UInt32) -> PlayerSlot { self }
  func load(key: String, route: PlaybackRoute) {}
  func unload() { lock.withLock { attached = false } }
  func preroll() {}
  func play() { plays.continuation.yield(lock.withLock { attached }) }
  func pause() {}
  func mute() {}
  func unmute() {}
  func attach(viewToken: UInt64) { lock.withLock { attached = viewToken == 42 } }
  func detach() { lock.withLock { attached = false } }
  func seek(positionMs: UInt64) {}
  func setRate(rate: Float) {}
  func setReadahead(ms: UInt64) {}
  func sample() -> SlotSample {
    SlotSample(positionMs: 0, durationMs: nil, isLooping: false, viewAttached: lock.withLock { attached }, frameRendered: false)
  }
}

private final class ReloadSettings: SettingsStore, @unchecked Sendable {
  func read(key: String) -> String? { nil }
  func write(key: String, value: String) {}
  func remove(key: String) {}
}

private final class ReloadFetcher: Fetcher, @unchecked Sendable {
  func fetch(request: FetchRequest) async throws -> FetchResponse { throw MediaError.Fetch(detail: "offline") }
}

@Suite(.timeLimit(.minutes(1)))
struct ReloadTests {
  @Test
  func inv_api_06_reload_restores_the_attached_view_before_playing() async throws {
    let slot = ReloadSlot()
    let media = try MediaLayer.open(config: OpenConfig(
      dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
      availableBytes: UInt64.max, fetcher: ReloadFetcher(), settings: ReloadSettings(), slots: slot,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(), pool: defaultPoolSettings()
    ))
    let key = "file:///clip.mp4"
    let events = media.playerEvents()
    events.attach(key: key, viewToken: 42)
    media.replaceWindow(active: 0, sources: [MediaItem(kind: .video, url: key, headers: [:], cover: nil, blurHash: nil, strategy: .window)])
    media.setPlaying(key: key, playing: true)
    var plays = slot.plays.stream.makeAsyncIterator()
    #expect(await plays.next() == true)
    events.failed(slot: 0, status: 0, detail: "decoder failed")
    #expect(await plays.next() == true)
  }
}
