import Foundation

#if os(iOS)
  import UIKit
#endif

enum ScreenHoldKind {
  case sound
  case always
}

/// One process counter of holders. A `sound` holder counts while the global mute is off, an `always` holder counts always. The system port is called inside the counter order when "any counted holder" flips.
final class ScreenAwake: @unchecked Sendable {
  static let shared = ScreenAwake(muted: Media.muted, apply: { value in
    DispatchQueue.main.async {
      #if os(iOS)
        if UIApplication.shared.isIdleTimerDisabled != value { UIApplication.shared.isIdleTimerDisabled = value }
      #endif
    }
  })

  private let lock = NSLock()
  private var port: (Bool) -> Void
  private var holders: [ObjectIdentifier: ScreenHoldKind] = [:]
  private var muted: Bool

  var apply: (Bool) -> Void {
    get { lock.withLock { port } }
    set { lock.withLock { port = newValue } }
  }

  init(muted: Bool = false, apply: @escaping (Bool) -> Void) {
    self.muted = muted
    port = apply
  }

  func set(_ holder: ObjectIdentifier, kind: ScreenHoldKind, on: Bool) {
    change { if on { holders[holder] = kind } else { holders[holder] = nil } }
  }

  func set(muted value: Bool) {
    change { muted = value }
  }

  private var counted: Bool { holders.values.contains { $0 == .always || !muted } }

  private func change(_ edit: () -> Void) {
    let flipped = lock.withLock { () -> Bool in
      let before = counted
      edit()
      let after = counted
      guard before != after else { return false }
      port(after)
      return true
    }
    if flipped { TestChange.fire() }
  }
}

/// One hold of one owner: a place (`sound`: while it plays and is not muted), a voice recorder, a camera or a call (`always`).
final class ScreenAwakeHold: @unchecked Sendable {
  private let awake: ScreenAwake
  private let kind: ScreenHoldKind
  private let lock = NSLock()
  private var playing = false
  private var muted = false

  init(kind: ScreenHoldKind = .always, awake: ScreenAwake = .shared) {
    self.kind = kind
    self.awake = awake
  }

  deinit {
    awake.set(ObjectIdentifier(self), kind: kind, on: false)
  }

  func set(playing value: Bool) {
    update { playing = value }
  }

  func set(muted value: Bool) {
    update { muted = value }
  }

  private func update(_ edit: () -> Void) {
    lock.withLock {
      edit()
      awake.set(ObjectIdentifier(self), kind: kind, on: playing && !muted)
    }
  }
}
