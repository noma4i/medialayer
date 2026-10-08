import Foundation
import MediaLayer
import QuartzCore
import Testing
@testable import MediaLayerUI

private final class CountingFetcher: Fetcher, @unchecked Sendable {
  private let lock = NSLock()
  private var calls = 0

  var count: Int { lock.withLock { calls } }

  func fetch(request: FetchRequest) async throws -> FetchResponse {
    lock.withLock { calls += 1 }
    throw MediaError.Fetch(detail: "offline")
  }
}

private final class StateLog: StateListener, @unchecked Sendable {
  private let lock = NSLock()
  private var states: [MediaState] = []

  var all: [MediaState] { lock.withLock { states } }

  func changed(state: MediaState) { lock.withLock { states.append(state) } }
}

private func eventually(_ condition: () -> Bool) async -> Bool {
  for _ in 0..<400 {
    if condition() { return true }
    try? await Task.sleep(for: .milliseconds(25))
  }
  return condition()
}

extension GlobalMedia {
  @Suite(.timeLimit(.minutes(1)))
  struct LocalClipPlaybackTests {
    private static var clip: String {
      Bundle.module.url(forResource: "silent", withExtension: "mp4", subdirectory: "Fixtures")!.absoluteString
    }

    private func open(silent: Bool, fetcher: Fetcher) throws -> MediaLayer {
      try MediaLayer.open(
        fetcher: fetcher,
        settings: UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!),
        directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString),
        silent: silent
      )
    }

    @Test
    func aLocalOneShotClipRendersAndEndsWithoutTheNetworkOrTheCache() async throws {
      let fetcher = CountingFetcher()
      let media = try open(silent: false, fetcher: fetcher)
      let log = StateLog()
      let subscription = media.subscribeState(key: Self.clip, listener: log)
      let item = MediaItem(kind: .video, url: Self.clip, headers: [:], cover: "https://o.test/cover.jpg", blurHash: nil, looping: false)
      let host = CALayer()
      let token = HostLayers.shared.register(host)

      media.visible(source: item, visible: true)
      media.setPlaying(key: Self.clip, playing: true)
      media.setMuted(key: Self.clip, muted: true)
      media.attachView(key: Self.clip, token: token)

      let ended = await eventually { log.all.last == .ended }
      #expect(ended)
      #expect(log.all.contains(.rendered))
      #expect(log.all.contains(.loading) == false)
      #expect(fetcher.count == 0)
      #expect(await media.stats().fileCount == 0)
      #expect(await media.placeholder(source: item, ready: false) == .none)
      withExtendedLifetime(subscription) {}
    }

    @Test
    func aSilentLayerKeepsEverySlotMutedWhateverThePlacesAsk() async throws {
      let media = try open(silent: true, fetcher: CountingFetcher())
      let item = MediaItem(kind: .video, url: Self.clip, headers: [:], cover: nil, blurHash: nil, looping: true)

      media.visible(source: item, visible: true)
      media.setPlaying(key: Self.clip, playing: true)
      let loaded = await eventually { LoadedSlots.shared.slot(for: Self.clip) != nil }
      #expect(loaded)
      let slot = try #require(LoadedSlots.shared.slot(for: Self.clip))
      await MainActor.run { slot.player.isMuted = false }

      media.setMutedAll(muted: true)
      media.setMutedAll(muted: false)
      media.setMuted(key: Self.clip, muted: false)

      try await Task.sleep(for: .milliseconds(300))
      #expect(await MainActor.run { slot.player.isMuted })
    }

    @Test
    func aRateSetOnTheLayerReachesThePlayerOfTheLoadedClip() async throws {
      let media = try open(silent: true, fetcher: CountingFetcher())
      let item = MediaItem(kind: .video, url: Self.clip, headers: [:], cover: nil, blurHash: nil, looping: true)

      media.setRate(key: Self.clip, rate: 1.5)
      media.visible(source: item, visible: true)

      let applied = await eventually { LoadedSlots.shared.slot(for: Self.clip)?.player.defaultRate == 1.5 }
      #expect(applied)
    }
  }
}
