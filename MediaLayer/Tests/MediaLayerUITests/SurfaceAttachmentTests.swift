import Foundation
import MediaLayer
import QuartzCore
import Testing
@testable import MediaLayerUI

private final class PlacesLog: MediaPlaces, @unchecked Sendable {
  private let counters = ImageCounters()
  func imageStats() -> ImageCounters { counters }
  func cacheKey(url: String, headers: [String: String]) -> String { url }
  private let lock = NSLock()
  private var entries: [String] = []

  var calls: [String] { lock.withLock { entries } }

  private func record(_ entry: String) { lock.withLock { entries.append(entry) } }

  func attachView(key: String, token: UInt64) { record("attach \(key) \(token)") }
  func detachView(key: String) { record("detach \(key)") }
  func visible(source: MediaItem, visible: Bool) {}
  func setPlaying(key: String, playing: Bool, groups: [String]) {}
  func setPeeking(key: String, peeking: Bool) {}
  func setMuted(key: String, muted: Bool) {}
  func setRate(key: String, rate: Float) {}
  func setMutedAll(muted: Bool) {}
  func seek(key: String, fraction: Double) {}
  func itemState(key: String, state: MediaState) {}
  func fetchImage(url: String, headers: [String: String]) async throws -> ImageFetchResult { ImageFetchResult(bytes: Data(), source: .network) }
  func placeholder(source: MediaItem, ready: Bool) async -> Placeholder { fatalError("unused") }
  func setTonePreparation(preparation: TonePreparation) {}
  func tonePixelsNeeded(source: MediaItem) async -> Bool { false }
  func playUrl(source: MediaItem) async throws -> PlaybackRoute { fatalError("unused") }
  func stateStream(key: String) -> AsyncStream<MediaState> { AsyncStream { $0.finish() } }
}

@Suite
struct SurfaceAttachmentTests {
  @Test
  func inv_native_20_open_objects_do_not_share_host_claims() {
    let first = PlacesLog()
    let second = PlacesLog()
    let key = UUID().uuidString
    let a = SurfaceAttachment(places: first, key: key, layer: olderLayer)
    let b = SurfaceAttachment(places: second, key: key, layer: newerLayer)
    #expect(first.calls.count == 1)
    #expect(second.calls.count == 1)
    withExtendedLifetime((a, b)) {}
  }

  private let places = PlacesLog()
  private let hosts = HostLayers()
  private let olderLayer = CALayer()
  private let newerLayer = CALayer()
  private let olderOwner = UUID()
  private let newerOwner = UUID()

  @Test
  func aNewHostOfAKeyDoesNotTakeTheLayerFromTheHostThatHasIt() {
    let older = SurfaceAttachment(places: places, key: "k", layer: olderLayer, owner: olderOwner, hosts: hosts)
    let newer = SurfaceAttachment(places: places, key: "k", layer: newerLayer, owner: newerOwner, hosts: hosts)

    #expect(places.calls == ["attach k \(older.token)"])
    withExtendedLifetime(newer) {}
  }

  @Test
  func inv_native_04_the_playing_owner_hands_the_view_back_when_it_leaves() {
    let older = SurfaceAttachment(places: places, key: "k", layer: olderLayer, owner: olderOwner, hosts: hosts)
    var newer: SurfaceAttachment? = SurfaceAttachment(places: places, key: "k", layer: newerLayer, owner: newerOwner, hosts: hosts)
    let newerToken = newer!.token

    KeyClaims(places: places, hosts: hosts).setPlaying(owner: newerOwner, key: "k", playing: true)
    #expect(places.calls.last == "attach k \(newerToken)")

    newer = nil
    #expect(places.calls.last == "attach k \(older.token)")
    withExtendedLifetime(older) {}
  }

  @Test
  func aNewHostThatIsAlreadyPlayingTakesTheLayer() {
    let older = SurfaceAttachment(places: places, key: "k", layer: olderLayer, owner: olderOwner, hosts: hosts)
    let newer = SurfaceAttachment(places: places, key: "k", layer: newerLayer, owner: newerOwner, isPlaying: true, hosts: hosts)

    #expect(places.calls == ["attach k \(older.token)", "attach k \(newer.token)"])
  }
}
