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
  var side: CGFloat = 50

  var body: some View {
    MediaImage(VisibilityGif(url: url)).environment(\.mediaContext, context).frame(width: side, height: side)
  }
}

private struct ContextCell: View {
  let places: RecordingPlaces
  let url: String
  var body: some View { MediaImage(VisibilityGif(url: url)).frame(width: 50, height: 50).mediaLayer(places) }
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
    func inv_native_22_replacing_open_object_changes_the_image_owner() async {
      let screen = Screen(gif: animatedGif(frames: 3, side: 64))
      defer { screen.close() }
      let second = RecordingPlaces()
      second.imageResult = .success(animatedGif(frames: 3, side: 64))
      ImageMemory.install(ImageCache(), on: second)
      let url = newURL()
      let host = UIHostingController(rootView: ContextCell(places: screen.places, url: url))
      screen.page.addChild(host)
      host.view.frame = CGRect(x: 0, y: 0, width: 50, height: 50)
      screen.scroll.addSubview(host.view)
      host.didMove(toParent: screen.page)
      await until("first owner fetched") { screen.places.fetchedURLs.count == 1 }
      host.rootView = ContextCell(places: second, url: url)
      await until("second owner fetched") { second.fetchedURLs.count == 1 }
      await until("first owner released") { screen.places.visibleCalls(of: url) == [true, false] }
      withExtendedLifetime(host) {}
    }

    @Test
    func inv_visibility_02_offscreen_video_does_not_claim_or_attach() async {
      let screen = Screen(gif: Data())
      defer { screen.close() }
      let url = newURL()
      let host = UIHostingController(rootView: MediaVideo(VisibilityGif(url: url)).environment(\.mediaContext, screen.context).frame(width: 50, height: 50))
      screen.page.addChild(host)
      host.view.frame = CGRect(x: 0, y: 1900, width: 50, height: 50)
      screen.scroll.addSubview(host.view)
      host.didMove(toParent: screen.page)
      await settle()
      #expect(screen.places.visibleCalls(of: url).isEmpty)
      #expect(!screen.places.recorded.contains("attach \(url)"))
      screen.scroll.contentOffset = CGPoint(x: 0, y: 1850)
      await until("video visible") { screen.places.visibleCalls(of: url) == [true] }
      await until("video attached") { screen.places.recorded.contains("attach \(url)") }
      withExtendedLifetime(host) {}
    }

    @Test
    func inv_image_34_same_pixel_size_keeps_prepared_request() async {
      let url = newURL()
      let screen = Screen(gif: animatedGif(frames: 3, side: 64))
      defer { screen.close() }
      let host = screen.mount(url, y: 0)
      await until("image fetched") { !screen.places.fetchedURLs.isEmpty }
      await settle()
      let reads = screen.places.cacheKeyReads
      host.rootView = GifCell(url: url, context: screen.context, side: 51)
      await settle()
      #expect(screen.places.cacheKeyReads == reads)
      withExtendedLifetime(host) {}
    }

    @Test
    func inv_image_35_returning_image_does_not_repeat_ready() async {
      let url = newURL()
      let screen = Screen(gif: animatedGif(frames: 3, side: 64))
      defer { screen.close() }
      let host = screen.mount(url, y: 0)
      await until("ready") { screen.places.recorded.contains("state ready") }
      screen.scroll.contentOffset = CGPoint(x: 0, y: 1000)
      await settle()
      screen.scroll.contentOffset = .zero
      await settle()
      #expect(screen.places.recorded.filter { $0 == "state ready" }.count == 1)
      withExtendedLifetime(host) {}
    }

    @Test
    func inv_image_33_offscreen_image_does_not_fetch() async {
      let screen = Screen(gif: animatedGif(frames: 3, side: 64))
      defer { screen.close() }
      let host = screen.mount(newURL(), y: 1900)
      await settle()
      #expect(screen.places.fetchedURLs.isEmpty)
      screen.scroll.contentOffset = CGPoint(x: 0, y: 1850)
      await until("visible image fetched") { screen.places.fetchedURLs.count == 1 }
      withExtendedLifetime(host) {}
    }

    @Test
    func inv_image_01_source_prepares_one_identity_despite_load_and_visibility_updates() async {
      let screen = Screen(gif: animatedGif(frames: 3, side: 64))
      defer { screen.close() }
      let before = screen.places.cacheKeyReads
      let host = screen.mount(newURL(), y: 0)
      await until("image fetched") { !screen.places.fetchedURLs.isEmpty }
      await settle()
      screen.window.isHidden = true
      await settle()
      screen.window.isHidden = false
      await settle()
      #expect(screen.places.cacheKeyReads - before == 1)
      withExtendedLifetime(host) {}
    }

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
    func an_image_fetch_scrolled_away_does_not_decode_until_it_returns() async {
      let url = newURL()
      let screen = Screen(gif: animatedGif(frames: 1200, side: 64))
      defer { screen.close() }
      await settle(0.3)
      let before = FrameCache.shared.decodes
      let latch = Latch()
      screen.places.fetchLatch = latch
      let host = screen.mount(url, y: 0)
      await until("the fetch starts") { screen.places.fetchedURLs.count == 1 }

      screen.scroll.contentOffset = CGPoint(x: 0, y: 1000)
      await until("the view leaves") { screen.places.visibleCalls(of: url) == [true, false] }
      latch.open()
      await settle()
      #expect(FrameCache.shared.decodes == before)
      screen.scroll.contentOffset = .zero

      await until("the returned view decodes") { FrameCache.shared.decodes == before + 1 }
      withExtendedLifetime(host) {}
    }

    @Test
    func two_views_of_one_gif_share_one_decode_and_each_releases_only_its_own_place() async {
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
      await until("the first view releases its own place") { screen.places.visibleCalls(of: url) == [true, true, false] }
      await settle()
      #expect(screen.places.visibleCalls(of: url) == [true, true, false])

      screen.unmount(second)
      await until("the second view releases its own place") { screen.places.visibleCalls(of: url) == [true, true, false, false] }
    }

    @Test
    func a_view_whose_url_changes_in_place_releases_the_old_place_and_leaves_the_place_of_its_other_viewer_alone() async {
      let old = newURL(), shared = newURL()
      let screen = Screen(gif: animatedGif(frames: 12, side: 64))
      defer { screen.close() }
      await settle(0.3)
      let other = screen.mount(shared, y: 50)
      let moving = screen.mount(old, y: 0)
      await until("both urls are declared") { screen.places.visibleCalls(of: old) == [true] && screen.places.visibleCalls(of: shared) == [true] }

      moving.rootView = GifCell(url: shared, context: screen.context)
      await settle()
      screen.unmount(moving)
      await settle()

      #expect(screen.places.visibleCalls(of: shared) == [true, true, false])
      #expect(screen.places.visibleCalls(of: old) == [true, false])
      withExtendedLifetime(other) {}
    }
  }
}
#endif
