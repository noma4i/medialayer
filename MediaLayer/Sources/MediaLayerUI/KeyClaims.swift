import Foundation
import MediaLayer
import ObjectiveC

nonisolated(unsafe) private var claimsKey: UInt8 = 0

final class KeyClaims: @unchecked Sendable {
  private struct Claim {
    var visible = false
    var playing = false
    var muted: Bool?
  }

  private struct Applied {
    var visible = false
    var playing = false
    var muted: Bool?
  }

  private let places: MediaPlaces
  private let hosts: HostLayers
  private let lock = NSLock()
  private var claims: [String: [UUID: Claim]] = [:]
  private var applied: [String: Applied] = [:]
  private var sources: [String: MediaItem] = [:]

  init(places: MediaPlaces, hosts: HostLayers = .shared) {
    self.places = places
    self.hosts = hosts
  }

  static func of(_ places: MediaPlaces) -> KeyClaims {
    if let existing = objc_getAssociatedObject(places, &claimsKey) as? KeyClaims { return existing }
    let created = KeyClaims(places: places)
    objc_setAssociatedObject(places, &claimsKey, created, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
    return created
  }

  func visible(owner: UUID, source: MediaItem, visible: Bool) {
    let key = source.url
    lock.withLock { if visible { sources[key] = source } }
    forward(update(owner: owner, key: key) { $0.visible = visible }, key: key)
  }

  func setPlaying(owner: UUID, key: String, playing: Bool) {
    forward(update(owner: owner, key: key) { $0.playing = playing }, key: key)
    if playing, let token = hosts.prefer(key: key, owner: owner) { places.attachView(key: key, token: token) }
  }

  func setMuted(owner: UUID, key: String, muted: Bool) {
    forward(update(owner: owner, key: key) { $0.muted = muted }, key: key)
  }

  func release(owner: UUID, key: String) {
    let change = lock.withLock { () -> Change in
      claims[key]?[owner] = nil
      if claims[key]?.isEmpty == true { claims[key] = nil }
      return diff(key: key)
    }
    forward(change, key: key)
  }

  private struct Change {
    var visible: Bool?
    var playing: Bool?
    var muted: Bool?
    var source: MediaItem?
  }

  private func update(owner: UUID, key: String, _ edit: (inout Claim) -> Void) -> Change {
    lock.withLock {
      var claim = claims[key]?[owner] ?? Claim()
      edit(&claim)
      claims[key, default: [:]][owner] = claim
      return diff(key: key)
    }
  }

  private func diff(key: String) -> Change {
    let all = claims[key]?.values.map { $0 } ?? []
    var last = applied[key] ?? Applied()
    var change = Change()
    let visible = all.contains { $0.visible }
    if visible != last.visible { change.visible = visible; last.visible = visible }
    let playing = all.contains { $0.playing }
    if playing != last.playing { change.playing = playing; last.playing = playing }
    let opinions = all.compactMap(\.muted)
    if !opinions.isEmpty {
      let muted = opinions.allSatisfy { $0 }
      if muted != last.muted { change.muted = muted; last.muted = muted }
    }
    applied[key] = all.isEmpty ? nil : last
    if all.isEmpty { change.source = sources.removeValue(forKey: key) } else { change.source = sources[key] }
    return change
  }

  private func forward(_ change: Change, key: String) {
    MediaTrace.emit("claims", "key=\(MediaTrace.tail(key)) visible=\(change.visible.map(String.init) ?? "-") playing=\(change.playing.map(String.init) ?? "-") muted=\(change.muted.map(String.init) ?? "-")")
    if change.visible == true, let source = change.source { places.visible(source: source, visible: true) }
    if let playing = change.playing { places.setPlaying(key: key, playing: playing) }
    if let muted = change.muted { places.setMuted(key: key, muted: muted) }
    if change.visible == false, let source = change.source { places.visible(source: source, visible: false) }
  }
}
