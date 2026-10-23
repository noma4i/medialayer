#if canImport(UIKit)
import CoreGraphics
import Foundation
import ImageIO
import MediaLayer
import Nuke
import SwiftUI
import Testing
import UIKit
import UniformTypeIdentifiers
@testable import MediaLayerUI

private struct GiphyLikeGif: MediaSource {
  let url: String
  var kind: Kind { .image }
  var headers: [String: String] { [:] }
  var size: MediaSize? { MediaSize(width: 64, height: 64) }
}

private struct StickerCell: View {
  let url: String
  let context: MediaContext

  var body: some View {
    MediaImage(GiphyLikeGif(url: url)).environment(\.mediaContext, context).frame(width: 50, height: 50)
  }
}

private final class CountingPlaces: MediaPlaces, @unchecked Sendable {
  func imageStats() -> ImageCounters { inner.imageStats() }
  func cacheKey(url: String, headers: [String: String]) -> String { url }
  let inner: any MediaPlaces
  private let lock = NSLock()
  private var reads = 0

  init(_ inner: any MediaPlaces) { self.inner = inner }

  var coreReads: Int { lock.withLock { reads } }

  func visible(source: MediaItem, visible: Bool) { inner.visible(source: source, visible: visible) }
  func setPlaying(key: String, playing: Bool, groups: [String?]) { inner.setPlaying(key: key, playing: playing, groups: groups) }
  func setPeeking(key: String, peeking: Bool) { inner.setPeeking(key: key, peeking: peeking) }
  func setMuted(key: String, muted: Bool) { inner.setMuted(key: key, muted: muted) }
  func setRate(key: String, rate: Float) { inner.setRate(key: key, rate: rate) }
  func setMutedAll(muted: Bool) { inner.setMutedAll(muted: muted) }
  func seek(key: String, fraction: Double) { inner.seek(key: key, fraction: fraction) }
  func itemState(key: String, state: MediaState) { inner.itemState(key: key, state: state) }
  func fetchImage(url: String, headers: [String: String]) async throws -> ImageFetchResult {
    lock.withLock { reads += 1 }
    return try await inner.fetchImage(url: url, headers: headers)
  }
  func placeholder(source: MediaItem, ready: Bool) async -> Placeholder { await inner.placeholder(source: source, ready: ready) }
  func setTonePreparation(preparation: TonePreparation) { inner.setTonePreparation(preparation: preparation) }
  func tonePixelsNeeded(source: MediaItem) async -> Bool { await inner.tonePixelsNeeded(source: source) }
  func playUrl(source: MediaItem) async throws -> PlaybackRoute { try await inner.playUrl(source: source) }
  func stateStream(key: String) -> AsyncStream<MediaState> { inner.stateStream(key: key) }
  func attachView(key: String, token: UInt64) { inner.attachView(key: key, token: token) }
  func detachView(key: String) { inner.detachView(key: key) }
}

private func stickerGif(frames: Int, side: Int) -> Data {
  let data = NSMutableData()
  let destination = CGImageDestinationCreateWithData(data, UTType.gif.identifier as CFString, frames, nil)!
  for index in 0 ..< frames {
    let context = CGContext(
      data: nil, width: side, height: side, bitsPerComponent: 8, bytesPerRow: 0,
      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    )!
    context.setFillColor(CGColor(red: CGFloat(index % 9) / 9, green: 0.2, blue: 1, alpha: 1))
    context.fill(CGRect(x: 0, y: 0, width: side, height: side))
    CGImageDestinationAddImage(destination, context.makeImage()!, [kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFDelayTime: 0.05]] as CFDictionary)
  }
  CGImageDestinationFinalize(destination)
  return data as Data
}

@MainActor
private func settle(_ seconds: Double) async {
  let end = Date().addingTimeInterval(seconds)
  while Date() < end {
    RunLoop.main.run(until: Date().addingTimeInterval(0.02))
    await Task.yield()
  }
}

@MainActor
private func until(_ condition: String, timeout: Double = 10, _ holds: () -> Bool) async {
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
  let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 100))
  let root = UIViewController()
  private(set) var hosts: [UIHostingController<StickerCell>] = []

  init() {
    window.rootViewController = root
    window.isHidden = false
  }

  func mount(_ url: String, context: MediaContext) {
    let host = UIHostingController(rootView: StickerCell(url: url, context: context))
    root.addChild(host)
    host.view.frame = CGRect(x: 0, y: CGFloat(hosts.count) * 50, width: 50, height: 50)
    host.view.backgroundColor = .clear
    host.safeAreaRegions = []
    root.view.addSubview(host.view)
    host.didMove(toParent: root)
    hosts.append(host)
  }

  func unmountAll() {
    for host in hosts {
      host.willMove(toParent: nil)
      host.view.removeFromSuperview()
      host.removeFromParent()
    }
    hosts = []
  }

  func close() {
    unmountAll()
    window.isHidden = true
    window.rootViewController = nil
  }
}

extension GlobalMedia {
  @MainActor
  @Suite(.timeLimit(.minutes(3)))
  struct GifReopenCounterTests {
    @Test
    func a_stable_giphy_style_url_costs_one_network_request_and_one_decode_per_context_and_a_restart_costs_a_decode_not_a_request() async throws {
      let server = try LocalHTTPServer()
      try await server.start()
      let path = "/media/abc123/200.gif"
      let gif = stickerGif(frames: 40, side: 64)
      server.serve(gif, at: path)
      let url = server.url(path)
      let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
      let settings = UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!)
      var media: MediaLayer? = try await MediaLayer.open(fetcher: URLSessionFetcher(), settings: settings, directory: directory)
      var places = CountingPlaces(try #require(media))
      ImageMemory.install(ImageCache(), on: places)
      var context = MediaContext(places: places)
      let screen = Screen()
      defer { screen.close() }
      let decodes0 = FrameCache.shared.decodes

      screen.mount(url, context: context)
      await until("first open decodes") { FrameCache.shared.decodes == decodes0 + 1 }
      await settle(0.5)
      let first = (network: server.requestCount(of: path), core: places.coreReads, decodes: FrameCache.shared.decodes - decodes0)
      #expect(first.network == 1 && first.core == 1 && first.decodes == 1, "cold open: \(first)")

      screen.mount(url, context: context)
      await settle(0.8)
      let repeated = (network: server.requestCount(of: path), core: places.coreReads, decodes: FrameCache.shared.decodes - decodes0)
      #expect(repeated.network == 1 && repeated.core == 1 && repeated.decodes == 1, "same url and size in a second view: \(repeated)")

      screen.unmountAll()
      await settle(0.3)
      screen.mount(url, context: context)
      await settle(0.8)
      let reopened = (network: server.requestCount(of: path), core: places.coreReads, decodes: FrameCache.shared.decodes - decodes0)
      #expect(reopened.network == 1 && reopened.core == 1 && reopened.decodes == 1, "screen reopened in the same context: \(reopened)")

      screen.unmountAll()
      server.stop()
      media = nil
      FrameCache.shared.removeAll()
      await settle(0.5)
      media = try await MediaLayer.open(fetcher: URLSessionFetcher(), settings: settings, directory: directory)
      places = CountingPlaces(try #require(media))
      ImageMemory.install(ImageCache(), on: places)
      context = MediaContext(places: places)
      let reopenedMedia = try #require(media)
      #expect(await reopenedMedia.isCached(source: GiphyLikeGif(url: url).item), "the persisted file must be visible right after open")
      let probe = try await places.fetchImage(url: url, headers: [:])
      #expect(probe.bytes == gif, "the first read after open comes from the disk, the origin is down")
      screen.mount(url, context: context)
      await until("restart decodes again") { FrameCache.shared.decodes == decodes0 + 2 }
      await settle(0.5)
      let restarted = (network: server.requestCount(of: path), core: places.coreReads, decodes: FrameCache.shared.decodes - decodes0)
      #expect(restarted.network == 1 && restarted.core == 2 && restarted.decodes == 2, "new context on the same directory, origin stopped: \(restarted)")
    }
  }
}
#endif
