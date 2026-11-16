#if canImport(UIKit)
import Foundation
import MediaLayer
import SwiftUI
import Testing
import UIKit
@testable import MediaLayerUI

private struct WakeClip: MediaSource {
  let url: String
  var kind: Kind { .audio }
  var headers: [String: String] { [:] }
}

/// A place of the core whose playback subscription the test drives, like the core pushes `PlaybackState`.
private final class DrivenPlace: PlaceHandle, @unchecked Sendable {
  private let lock = NSLock()
  private var listener: (any PlaybackListener)?
  private var isReleased = false

  var subscribed: Bool { lock.withLock { listener != nil } }
  var released: Bool { lock.withLock { isReleased } }

  func push(playing: Bool) {
    lock.withLock { listener }?.changed(state: PlaybackState(playing: playing, wish: playing, reason: nil, refused: nil, attempt: 0))
  }

  func subscribePlayback(listener: any PlaybackListener) throws -> Subscription {
    lock.withLock { self.listener = listener }
    TestSignal.shared.fire()
    return Subscription(noHandle: .init())
  }
  func update(input: PlaceInput) throws {}
  func seek(fraction: Double) throws {}
  func itemState(state: MediaState) throws {}
  func release() {
    lock.withLock { isReleased = true }
    TestSignal.shared.fire()
  }
}

private struct WakeCell: View {
  let context: MediaContext

  var body: some View {
    MediaAudio(WakeClip(url: "file:///wake.m4a")) { _ in Color.clear }
      .frame(width: 40, height: 40)
      .environment(\.mediaContext, context)
  }
}

/// Every delivery queued on Main before this call has run when it returns.
@MainActor
private func queuedDeliveriesRan() async {
  await Task { @MainActor in }.value
}

extension GlobalMedia {
  @MainActor
  @Suite(.serialized, .timeLimit(.minutes(1)))
  struct ScreenWakeTests {
    /// The system port of the screen flag: what the library asks the system to do.
    @MainActor
    private final class ScreenPort {
      var on = false
      private let original: (Bool) -> Void
      init() {
        original = ScreenAwake.shared.apply
        ScreenAwake.shared.apply = { [unowned self] value in on = value }
      }
      func restore() { ScreenAwake.shared.apply = original }
    }

    private func mount(_ place: DrivenPlace) async -> UIWindow {
      let places = RecordingPlaces()
      places.placeFactory = { _ in place }
      let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 40, height: 40))
      window.rootViewController = UIHostingController(rootView: WakeCell(context: MediaContext(places: places)))
      window.isHidden = false
      await eventually("the place subscribed to its playback") { place.subscribed }
      return window
    }

    private func unmount(_ window: UIWindow) {
      window.isHidden = true
      window.rootViewController = nil
    }

    @Test
    func inv_api_60_a_playing_place_keeps_the_screen_on() async {
      let screen = ScreenPort()
      defer { screen.restore() }
      let place = DrivenPlace()
      let window = await mount(place)
      defer { unmount(window) }
      #expect(!screen.on)

      place.push(playing: true)
      await eventually("the screen stays on while the place plays") { screen.on }

      place.push(playing: false)
      await eventually("the screen may sleep after the place stops") { !screen.on }
    }

    @Test
    func inv_api_61_a_late_delivery_after_release_does_not_keep_the_screen_on() async {
      let screen = ScreenPort()
      defer { screen.restore() }
      let place = DrivenPlace()
      let window = await mount(place)
      unmount(window)
      await eventually("the place is released") { place.released }

      place.push(playing: true)
      await queuedDeliveriesRan()

      #expect(!screen.on)
    }
  }
}
#endif
