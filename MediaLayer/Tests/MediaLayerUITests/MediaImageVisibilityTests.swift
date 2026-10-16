#if canImport(UIKit)
import CoreGraphics
import ImageIO
import MediaLayer
import Nuke
import SwiftUI
import Testing
import UIKit
import UniformTypeIdentifiers
@testable import MediaLayerUI

private struct VisibilityGif: MediaSource {
  let url: String
  var kind: Kind { .image }
  var headers: [String: String] { [:] }
  var size: MediaSize? { MediaSize(width: 64, height: 64) }
}

private struct GifCell: View {
  let url: String
  let context: MediaContext

  var body: some View {
    MediaImage(VisibilityGif(url: url)).environment(\.mediaContext, context).frame(width: 50, height: 50)
  }
}

private func animatedGif(frames: Int, side: Int) -> Data {
  let data = NSMutableData()
  let destination = CGImageDestinationCreateWithData(data, UTType.gif.identifier as CFString, frames, nil)!
  for index in 0 ..< frames {
    let context = CGContext(
      data: nil, width: side, height: side, bitsPerComponent: 8, bytesPerRow: 0,
      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    )!
    context.setFillColor(CGColor(red: CGFloat(index % 7) / 7, green: 0, blue: 1, alpha: 1))
    context.fill(CGRect(x: 0, y: 0, width: side, height: side))
    CGImageDestinationAddImage(destination, context.makeImage()!, [kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFDelayTime: 0.1]] as CFDictionary)
  }
  CGImageDestinationFinalize(destination)
  return data as Data
}

@MainActor
private func settle(_ seconds: Double = 0.4) async {
  let end = Date().addingTimeInterval(seconds)
  while Date() < end {
    RunLoop.main.run(until: Date().addingTimeInterval(0.02))
    await Task.yield()
  }
}

@MainActor
private func until(_ condition: String, timeout: Double = 5, _ holds: () -> Bool) async {
  let end = Date().addingTimeInterval(timeout)
  while !holds() {
    guard Date() < end else {
      Issue.record("never became true within \(timeout)s: \(condition)")
      return
    }
    RunLoop.main.run(until: Date().addingTimeInterval(0.02))
    await Task.yield()
  }
}

@MainActor
private final class Screen {
  let places = RecordingPlaces()
  let context: MediaContext
  let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 100))
  let navigation: UINavigationController
  let page = UIViewController()
  let scroll = UIScrollView(frame: CGRect(x: 0, y: 0, width: 200, height: 100))

  init(gif: Data) {
    places.imageResult = .success(gif)
    ImageMemory.install(ImageCache(), on: places)
    context = MediaContext(places: places)
    scroll.contentSize = CGSize(width: 200, height: 2000)
    scroll.contentInsetAdjustmentBehavior = .never
    page.view.addSubview(scroll)
    navigation = UINavigationController(rootViewController: page)
    navigation.setNavigationBarHidden(true, animated: false)
    window.rootViewController = navigation
    window.isHidden = false
    page.view.frame = window.bounds
    scroll.frame = window.bounds
  }

  func close() {
    window.isHidden = true
    window.rootViewController = nil
  }

  func mount(_ url: String, y: CGFloat) -> UIHostingController<GifCell> {
    let host = UIHostingController(rootView: GifCell(url: url, context: context))
    page.addChild(host)
    host.view.frame = CGRect(x: 0, y: y, width: 50, height: 50)
    host.view.backgroundColor = .clear
    host.safeAreaRegions = []
    scroll.addSubview(host.view)
    host.didMove(toParent: page)
    return host
  }

  func unmount(_ host: UIHostingController<GifCell>) {
    host.willMove(toParent: nil)
    host.view.removeFromSuperview()
    host.removeFromParent()
  }
}

private func newURL() -> String { "https://o.test/\(UUID().uuidString).gif" }

extension GlobalMedia {
  @MainActor
  @Suite(.timeLimit(.minutes(3)))
  struct MediaImageVisibilityTests {
    @Test
    func a_gif_clipped_out_of_its_scroll_view_neither_claims_its_url_nor_decodes_until_it_scrolls_in() async {
      let url = newURL()
      let screen = Screen(gif: animatedGif(frames: 12, side: 64))
      defer { screen.close() }
      await settle(0.3)
      let before = FrameCache.shared.decodes

      let host = screen.mount(url, y: 1900)
      await settle()
      #expect(FrameCache.shared.decodes == before)
      #expect(screen.places.visibleCalls(of: url) == [])

      screen.scroll.contentOffset = CGPoint(x: 0, y: 1850)
      await until("the decode starts once the view is in the clip") { FrameCache.shared.decodes == before + 1 }
      #expect(screen.places.visibleCalls(of: url) == [true])
      withExtendedLifetime(host) {}
    }

    @Test
    func a_gif_in_a_hidden_window_releases_its_claim_and_decodes_only_after_the_window_returns() async {
      let url = newURL()
      let screen = Screen(gif: animatedGif(frames: 12, side: 64))
      defer { screen.close() }
      await settle(0.3)
      let latch = Latch()
      screen.places.fetchLatch = latch
      let before = FrameCache.shared.decodes
      let host = screen.mount(url, y: 0)
      await until("the view claims its url") { screen.places.visibleCalls(of: url) == [true] }

      screen.window.isHidden = true
      await until("the claim is released") { screen.places.visibleCalls(of: url) == [true, false] }
      latch.open()
      await settle()
      #expect(FrameCache.shared.decodes == before)

      screen.window.isHidden = false
      await until("the decode starts when the window returns") { FrameCache.shared.decodes == before + 1 }
      withExtendedLifetime(host) {}
    }

    @Test
    func a_gif_under_a_hidden_ancestor_releases_its_claim_and_decodes_only_after_the_ancestor_returns() async {
      let url = newURL()
      let screen = Screen(gif: animatedGif(frames: 12, side: 64))
      defer { screen.close() }
      await settle(0.3)
      let latch = Latch()
      screen.places.fetchLatch = latch
      let before = FrameCache.shared.decodes
      let host = screen.mount(url, y: 0)
      await until("the view claims its url") { screen.places.visibleCalls(of: url) == [true] }

      screen.scroll.isHidden = true
      await until("the claim is released") { screen.places.visibleCalls(of: url) == [true, false] }
      latch.open()
      await settle()
      #expect(FrameCache.shared.decodes == before)

      screen.scroll.isHidden = false
      await until("the decode starts when the ancestor returns") { FrameCache.shared.decodes == before + 1 }
      withExtendedLifetime(host) {}
    }

    @Test
    func a_gif_in_a_backgrounded_app_does_not_decode_and_decodes_when_the_app_returns() async {
      let url = newURL()
      let screen = Screen(gif: animatedGif(frames: 12, side: 64))
      defer { screen.close() }
      await settle(0.3)
      let latch = Latch()
      screen.places.fetchLatch = latch
      let before = FrameCache.shared.decodes
      let host = screen.mount(url, y: 0)
      await until("the view claims its url") { screen.places.visibleCalls(of: url) == [true] }

      NotificationCenter.default.post(name: UIApplication.didEnterBackgroundNotification, object: nil)
      latch.open()
      await settle()
      #expect(FrameCache.shared.decodes == before)
      #expect(screen.places.visibleCalls(of: url) == [true])

      NotificationCenter.default.post(name: UIApplication.willEnterForegroundNotification, object: nil)
      await until("the decode starts when the app returns") { FrameCache.shared.decodes == before + 1 }
      withExtendedLifetime(host) {}
    }

    @Test
    func a_gif_covered_by_a_pushed_screen_stops_and_decodes_again_after_the_pop() async {
      let url = newURL()
      let screen = Screen(gif: animatedGif(frames: 12, side: 64))
      defer { screen.close() }
      await settle(0.3)
      let latch = Latch()
      screen.places.fetchLatch = latch
      let before = FrameCache.shared.decodes
      let host = screen.mount(url, y: 0)
      await until("the view claims its url") { screen.places.visibleCalls(of: url) == [true] }

      screen.navigation.pushViewController(UIViewController(), animated: false)
      await until("the claim is released") { screen.places.visibleCalls(of: url) == [true, false] }
      latch.open()
      await settle()
      #expect(FrameCache.shared.decodes == before)

      screen.navigation.popViewController(animated: false)
      await until("the decode starts after the pop") { FrameCache.shared.decodes == before + 1 }
      withExtendedLifetime(host) {}
    }

    @Test
    func a_transparent_screen_over_a_gif_leaves_it_claimed_and_decoding() async {
      let url = newURL()
      let screen = Screen(gif: animatedGif(frames: 12, side: 64))
      defer { screen.close() }
      await settle(0.3)
      let before = FrameCache.shared.decodes
      let host = screen.mount(url, y: 0)
      await until("the view claims its url") { screen.places.visibleCalls(of: url) == [true] }

      let cover = UIViewController()
      cover.view.backgroundColor = .clear
      cover.modalPresentationStyle = .overFullScreen
      screen.navigation.present(cover, animated: false)
      await settle()

      #expect(screen.places.visibleCalls(of: url) == [true])
      await until("the decode runs under the transparent screen") { FrameCache.shared.decodes == before + 1 }
      withExtendedLifetime(host) {}
    }

    @Test
    func a_decode_in_flight_is_cancelled_when_its_view_scrolls_away_and_starts_again_when_it_returns() async {
      let url = newURL()
      let screen = Screen(gif: animatedGif(frames: 1200, side: 64))
      defer { screen.close() }
      await settle(0.3)
      let before = FrameCache.shared.decodes
      let host = screen.mount(url, y: 0)
      await until("the decode starts") { FrameCache.shared.decodes == before + 1 }

      screen.scroll.contentOffset = CGPoint(x: 0, y: 1000)
      await settle(1)
      screen.scroll.contentOffset = .zero

      await until("the cancelled decode is not cached, so the return decodes again") { FrameCache.shared.decodes == before + 2 }
      withExtendedLifetime(host) {}
    }

    @Test
    func two_views_of_one_gif_share_one_decode_and_the_url_stays_claimed_until_the_last_view_leaves() async {
      let url = newURL()
      let screen = Screen(gif: animatedGif(frames: 1200, side: 64))
      defer { screen.close() }
      await settle(0.3)
      let before = FrameCache.shared.decodes
      let first = screen.mount(url, y: 0)
      let second = screen.mount(url, y: 50)
      await until("a decode starts") { FrameCache.shared.decodes > before }
      await settle()
      #expect(FrameCache.shared.decodes == before + 1)

      screen.unmount(first)
      await settle()
      #expect(screen.places.visibleCalls(of: url) == [true])

      screen.unmount(second)
      await until("the last view releases the url") { screen.places.visibleCalls(of: url) == [true, false] }
    }

    @Test
    func a_view_whose_url_changes_in_place_hands_the_old_url_back_and_never_withdraws_the_new_one_from_its_other_viewer() async {
      let old = newURL(), shared = newURL()
      let screen = Screen(gif: animatedGif(frames: 12, side: 64))
      defer { screen.close() }
      await settle(0.3)
      let other = screen.mount(shared, y: 50)
      let moving = screen.mount(old, y: 0)
      await until("both urls are claimed") { screen.places.visibleCalls(of: old) == [true] && screen.places.visibleCalls(of: shared) == [true] }

      moving.rootView = GifCell(url: shared, context: screen.context)
      await settle()
      screen.unmount(moving)
      await settle()

      #expect(screen.places.visibleCalls(of: shared) == [true])
      #expect(screen.places.visibleCalls(of: old) == [true, false])
      withExtendedLifetime(other) {}
    }
  }
}
#endif
