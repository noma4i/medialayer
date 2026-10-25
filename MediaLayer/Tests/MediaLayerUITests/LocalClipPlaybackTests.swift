import Foundation
import MediaLayer
import MediaLayerTesting
import QuartzCore
import Testing
@testable import MediaLayerUI
#if canImport(UIKit)
import UIKit
#endif

private final class StateLog: StateListener, @unchecked Sendable {
  private let lock = NSLock()
  private var states: [MediaState] = []

  var all: [MediaState] { lock.withLock { states } }

  func changed(state: MediaState) { lock.withLock { states.append(state) } }

  func failed(error: MediaError) {}
}

private func eventually(_ condition: () -> Bool) async -> Bool {
  for _ in 0..<400 {
    if condition() { return true }
    try? await Task.sleep(for: .milliseconds(25))
  }
  return condition()
}

extension GlobalMedia {
  @Suite(.serialized, .timeLimit(.minutes(1)))
  struct LocalClipPlaybackTests {
    private static var clip: String {
      Bundle.module.url(forResource: "silent", withExtension: "mp4", subdirectory: "Fixtures")!.absoluteString
    }

    private func open(silent: Bool, fetcher: Fetcher) async throws -> MediaLayer {
      try await MediaLayer.open(
        fetcher: fetcher,
        settings: UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!),
        directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString),
        silent: silent
      )
    }

    private func release(_ media: MediaLayer) async {
      media.releaseOwned()
      await eventually("the clip leaves the shared slots") { LoadedSlots.of(media).slot(for: Self.clip) == nil }
    }

    #if canImport(UIKit)
    @MainActor
    @Test
    func aLocalOneShotClipRendersAndEndsWithoutTheNetworkOrTheCache() async throws {
      let fetcher = TestFetcher()
      let media = try await open(silent: false, fetcher: fetcher)
      let log = StateLog()
      let subscription = try media.subscribeState(key: Self.clip, listener: log)
      let item = MediaItem(kind: .video, url: Self.clip, headers: [:], cover: "https://o.test/cover.jpg", blurHash: nil, looping: false, strategy: .window)
      let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 64, height: 64))
      let controller = UIViewController()
      window.rootViewController = controller
      window.isHidden = false
      let host = controller.view.layer
      let token = HostLayers.of(media).register(host)
      defer {
        media.releaseOwned()
        HostLayers.of(media).unregister(token)
        window.isHidden = true
        window.rootViewController = nil
      }

      media.visible(source: item, visible: true)
      media.setPlaying(key: Self.clip, playing: true)
      media.setMuted(key: Self.clip, muted: true)
      media.attachView(key: Self.clip, token: token)

      let ended = await eventually { log.all.last == .ended }
      #expect(ended)
      #expect(log.all.contains(.rendered))
      #expect(log.all.contains(.loading) == false)
      #expect(fetcher.requests.isEmpty)
      #expect(try await media.stats().fileCount == 0)
      #expect(await media.placeholder(source: item, ready: false) == .none)
      withExtendedLifetime(subscription) {}
      await release(media)
    }
    #endif

    @Test
    func aSilentLayerKeepsEverySlotMutedWhateverThePlacesAsk() async throws {
      let media = try await open(silent: true, fetcher: TestFetcher())
      let item = MediaItem(kind: .video, url: Self.clip, headers: [:], cover: nil, blurHash: nil, looping: true, strategy: .window)

      media.visible(source: item, visible: true)
      media.setPlaying(key: Self.clip, playing: true)
      let loaded = await eventually { LoadedSlots.of(media).slot(for: Self.clip) != nil }
      #expect(loaded)
      let slot = try #require(LoadedSlots.of(media).slot(for: Self.clip))

      func unmuteOutsideThenAsk(_ ask: () -> Void) async -> Bool {
        await MainActor.run { slot.player.isMuted = false }
        ask()
        for _ in 0..<400 {
          if await MainActor.run(body: { slot.player.isMuted }) { return true }
          try? await Task.sleep(for: .milliseconds(25))
        }
        return false
      }

      #expect(await unmuteOutsideThenAsk { try! media.setMutedAll(muted: true) })
      #expect(await unmuteOutsideThenAsk { try! media.setMutedAll(muted: false) })
      #expect(await unmuteOutsideThenAsk { media.setMuted(key: Self.clip, muted: true) })
      #expect(await unmuteOutsideThenAsk { media.setMuted(key: Self.clip, muted: false) })
      await release(media)
    }

    @Test
    func aClaimToUnmuteReachesASlotOfANonSilentLayer() async throws {
      let media = try await MediaLayer.open(
        fetcher: TestFetcher(),
        settings: UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!),
        directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString),
        silent: false
      )
      let item = MediaItem(kind: .video, url: Self.clip, headers: [:], cover: nil, blurHash: nil, looping: true, strategy: .window)

      media.visible(source: item, visible: true)
      media.setMuted(key: Self.clip, muted: false)
      media.setPlaying(key: Self.clip, playing: true)

      let unmuted = await eventually {
        guard let slot = LoadedSlots.of(media).slot(for: Self.clip) else { return false }
        return !slot.player.isMuted
      }
      #expect(unmuted)
      await release(media)
    }

    @Test
    func aRateSetOnTheLayerReachesThePlayerOfTheLoadedClip() async throws {
      let media = try await open(silent: true, fetcher: TestFetcher())
      let item = MediaItem(kind: .video, url: Self.clip, headers: [:], cover: nil, blurHash: nil, looping: true, strategy: .window)

      media.setRate(key: Self.clip, rate: 1.5)
      media.visible(source: item, visible: true)

      let applied = await eventually { LoadedSlots.of(media).slot(for: Self.clip)?.player.defaultRate == 1.5 }
      #expect(applied)
      await release(media)
    }
  }
}
