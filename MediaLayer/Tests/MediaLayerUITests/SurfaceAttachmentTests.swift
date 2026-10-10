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
  @Test
  func theHostThatLostTheKeyGetsItBackWhenTheNewerHostLeaves() {
    let places = PlacesLog()
    let hosts = HostLayers()
    let olderLayer = CALayer()
    let newerLayer = CALayer()
    var older: SurfaceAttachment? = SurfaceAttachment(places: places, key: "k", layer: olderLayer, hosts: hosts)
    let olderToken = older!.token
    var newer: SurfaceAttachment? = SurfaceAttachment(places: places, key: "k", layer: newerLayer, hosts: hosts)
    let newerToken = newer!.token

    newer = nil

    #expect(places.calls == ["attach k \(olderToken)", "attach k \(newerToken)", "attach k \(olderToken)"])
    older = nil
    #expect(places.calls.last == "detach k")
    withExtendedLifetime((olderLayer, newerLayer)) {}
  }

  @Test
  func anOlderHostLeavingKeepsTheNewerHostAttached() {
    let places = PlacesLog()
    let hosts = HostLayers()
    let olderLayer = CALayer()
    let newerLayer = CALayer()
    var older: SurfaceAttachment? = SurfaceAttachment(places: places, key: "k", layer: olderLayer, hosts: hosts)
    let newer = SurfaceAttachment(places: places, key: "k", layer: newerLayer, hosts: hosts)

    older = nil

    #expect(places.calls.count == 2)
    withExtendedLifetime((newer, olderLayer, newerLayer)) {}
  }
}
