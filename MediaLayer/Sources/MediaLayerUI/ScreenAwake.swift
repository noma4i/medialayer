import Foundation

#if os(iOS)
  import UIKit
#endif

@MainActor
final class ScreenAwake {
  static let shared = ScreenAwake(apply: { value in
    #if os(iOS)
      if UIApplication.shared.isIdleTimerDisabled != value { UIApplication.shared.isIdleTimerDisabled = value }
    #endif
  })

  var apply: (Bool) -> Void
  private var holders = Set<ObjectIdentifier>()

  init(apply: @escaping (Bool) -> Void) {
    self.apply = apply
  }

  func set(_ holder: ObjectIdentifier, playing: Bool) {
    let before = !holders.isEmpty
    if playing { holders.insert(holder) } else { holders.remove(holder) }
    let after = !holders.isEmpty
    guard before != after else { return }
    apply(after)
    TestChange.fire()
  }
}

@MainActor
final class ScreenAwakeHold {
  private let awake: ScreenAwake

  init(awake: ScreenAwake = .shared) {
    self.awake = awake
  }

  func set(playing: Bool) {
    awake.set(ObjectIdentifier(self), playing: playing)
  }
}
