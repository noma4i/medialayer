import Foundation
import MediaLayer
import ObjectiveC

nonisolated(unsafe) private var drivingKey: UInt8 = 0

/// Test driver: gives a layer the per-key calls the tests read well with, over the owned places and windows of the library.
final class OwnedDriving: @unchecked Sendable {
  private let lock = NSLock()
  private var places: [String: MediaPlace] = [:]
  private var inputs: [String: PlaceInput] = [:]
  private var video: MediaWindow?
  private var images: [String: MediaWindow] = [:]

  static func of(_ media: MediaLayer) -> OwnedDriving {
    objc_sync_enter(media)
    defer { objc_sync_exit(media) }
    if let existing = objc_getAssociatedObject(media, &drivingKey) as? OwnedDriving { return existing }
    let created = OwnedDriving()
    objc_setAssociatedObject(media, &drivingKey, created, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
    return created
  }

  fileprivate func edit(_ media: MediaLayer, key: String, item: MediaItem? = nil, _ change: (inout PlaceInput) -> Void) {
    lock.withLock {
      if places[key] == nil {
        places[key] = try! media.openPlace(item: item ?? MediaItem(kind: .video, url: key, headers: [:], cover: nil, blurHash: nil, looping: true, strategy: .window))
        inputs[key] = PlaceInput(visible: false, playing: false, group: nil, peeking: false, muted: nil, rate: nil, pinned: false, viewToken: nil)
      }
      var input = inputs[key]!
      change(&input)
      inputs[key] = input
      try! places[key]!.update(input: input)
    }
  }

  fileprivate func place(_ key: String) -> MediaPlace? { lock.withLock { places[key] } }

  fileprivate func drop(_ key: String) {
    lock.withLock {
      places[key]?.release()
      places[key] = nil
      inputs[key] = nil
    }
  }

  fileprivate func videoWindow(_ media: MediaLayer) -> MediaWindow {
    lock.withLock {
      if video == nil { video = try! media.openVideoWindow() }
      return video!
    }
  }

  fileprivate func imageWindow(_ media: MediaLayer, _ name: String) -> MediaWindow {
    lock.withLock {
      if images[name] == nil { images[name] = try! media.openImageWindow(name: name) }
      return images[name]!
    }
  }

  fileprivate func releaseAll() {
    lock.withLock {
      places.values.forEach { $0.release() }
      places = [:]
      inputs = [:]
      video?.release()
      video = nil
      images.values.forEach { $0.release() }
      images = [:]
    }
  }
}

extension MediaLayer {
  var driving: OwnedDriving { OwnedDriving.of(self) }

  func visible(source: MediaItem, visible: Bool) {
    if visible { driving.edit(self, key: source.url, item: source) { $0.visible = true } } else { driving.drop(source.url) }
  }

  func setPlaying(key: String, playing: Bool, groups: [String?] = []) {
    driving.edit(self, key: key) {
      $0.playing = playing
      $0.group = groups.compactMap { $0 }.first
    }
  }

  func setPeeking(key: String, peeking: Bool) { driving.edit(self, key: key) { $0.peeking = peeking } }

  func setMuted(key: String, muted: Bool) { driving.edit(self, key: key) { $0.muted = muted } }

  func setRate(key: String, rate: Float) { driving.edit(self, key: key) { $0.rate = rate } }

  func attachView(key: String, token: UInt64) { driving.edit(self, key: key) { $0.viewToken = token } }

  func detachView(key: String) { driving.edit(self, key: key) { $0.viewToken = nil } }

  func seek(key: String, fraction: Double) {
    driving.edit(self, key: key) { _ in }
    try! driving.place(key)?.seek(fraction: fraction)
  }

  func itemState(key: String, state: MediaState) {
    driving.edit(self, key: key) { _ in }
    try! driving.place(key)?.itemState(state: state)
  }

  func replaceWindow(active: UInt32, sources: [MediaItem?]) { try! driving.videoWindow(self).replace(active: active, sources: sources) }

  func moveWindow(active: UInt32) { try! driving.videoWindow(self).moveTo(active: active) }

  func replaceImageWindow(name: String, active: UInt32, pages: [[MediaItem]]) { try! driving.imageWindow(self, name).replacePages(active: active, pages: pages) }

  func replaceImageWindow<Source: MediaSource>(_ name: String, active: Int, pages: [[Source]]) {
    replaceImageWindow(name: name, active: UInt32(clamping: max(active, 0)), pages: pages.map { $0.map(\.item) })
  }

  func releaseOwned() {
    driving.releaseAll()
    try! releaseAll()
  }
}
