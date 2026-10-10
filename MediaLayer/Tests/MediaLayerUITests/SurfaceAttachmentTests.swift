import Foundation
import MediaLayer
import QuartzCore
import Testing
@testable import MediaLayerUI

private final class PlacesLog: MediaPlaces, @unchecked Sendable {
  private let lock = NSLock()
  private var entries: [String] = []

  var calls: [String] { lock.withLock { entries } }

  private func record(_ entry: String) { lock.withLock { entries.append(entry) } }

  func attachView(key: String, token: UInt64) { record("attach \(key) \(token)") }
  func detachView(key: String) { record("detach \(key)") }
  func visible(source: MediaItem, visible: Bool) {}
  func setPlaying(key: String, playing: Bool) {}
  func setMuted(key: String, muted: Bool) {}
  func setRate(key: String, rate: Float) {}
  func setMutedAll(muted: Bool) {}
  func seek(key: String, positionMs: UInt64) {}
  func itemState(key: String, state: MediaState) {}
  func fetchImage(url: String, headers: [String: String]) async throws -> Data { Data() }
  func placeholder(source: MediaItem, ready: Bool) async -> Placeholder { fatalError("unused") }
  func stateStream(key: String) -> AsyncStream<MediaState> { AsyncStream { $0.finish() } }
}

@Suite
struct SurfaceAttachmentTests {
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
  func theHostWhoseOwnerStartsPlayingTakesTheLayerAndHandsItBackWhenItLeaves() {
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
