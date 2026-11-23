#if canImport(UIKit)
import MediaLayer
import Nuke
import SwiftUI
import Testing
import UIKit
@testable import MediaLayerUI

private struct Photo: MediaSource {
  let url: String
  var kind: Kind { .image }
  var headers: [String: String] { [:] }
  var size: MediaSize? { MediaSize(width: 32, height: 32) }
}

private struct SizedPhoto: MediaSource {
  let url: String
  var kind: Kind { .image }
  var headers: [String: String] { [:] }
  var size: MediaSize? { nil }
}

private func solid(_ color: UIColor) -> Data {
  let format = UIGraphicsImageRendererFormat()
  format.scale = 1
  return UIGraphicsImageRenderer(size: CGSize(width: 32, height: 32), format: format).pngData { context in
    color.setFill()
    context.fill(CGRect(x: 0, y: 0, width: 32, height: 32))
  }
}

@MainActor
private func centerPixel(of host: UIView) -> [UInt8] {
  var pixel = [UInt8](repeating: 0, count: 4)
  let context = CGContext(
    data: &pixel, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
    space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
  )!
  context.translateBy(x: -host.bounds.midX, y: -host.bounds.midY)
  host.layer.render(in: context)
  return pixel
}

@MainActor
private func until(_ condition: String, _ holds: () -> Bool) async {
  let end = Date().addingTimeInterval(5)
  while !holds() {
    guard Date() < end else {
      Issue.record("never became true within 5s: \(condition)")
      return
    }
    RunLoop.main.run(until: Date().addingTimeInterval(0.02))
    await Task.yield()
  }
}

@MainActor
private final class Switch: ObservableObject {
  @Published var url: String
  init(url: String) { self.url = url }
}

private struct Switching: View {
  @ObservedObject var model: Switch
  let preview: Photo

  var body: some View { MediaImage(Photo(url: model.url), preview: preview) }
}

@MainActor
private func backdropIdentifiers(_ object: Any?) -> [String] {
  guard let object else { return [] }
  var found: [String] = []
  if let node = object as? NSObject, node.responds(to: Selector(("accessibilityIdentifier"))), let id = node.value(forKey: "accessibilityIdentifier") as? String { found.append(id) }
  if let view = object as? UIView {
    for element in view.accessibilityElements ?? [] { found += backdropIdentifiers(element) }
    for child in view.subviews { found += backdropIdentifiers(child) }
  } else if let node = object as? NSObject {
    let count = node.accessibilityElementCount()
    if count != NSNotFound { for index in 0..<count { found += backdropIdentifiers(node.accessibilityElement(at: index)) } }
  }
  return found
}

extension GlobalMedia {
  @MainActor
  @Suite(.timeLimit(.minutes(3)))
  struct MediaImagePreviewTests {
    @Test
    func inv_image_88_first_frame_uses_the_cached_size_until_the_new_size_loads() async throws {
      let places = RecordingPlaces()
      ImageMemory.install(ImageCache(), on: places)
      let context = MediaContext(places: places)
      let photo = SizedPhoto(url: "https://o.test/sized-\(UUID().uuidString).png")
      places.answer(photo.url, with: solid(.red))
      let request = try #require(await MediaImagePipeline.prepareOffMain(places: places, for: photo, points: CGSize(width: 20, height: 20), scale: 1).request)
      let initial = await MediaImagePipeline.load(photo, request: request, pipeline: context.pipeline, index: context.stillImages)
      #expect(initial != nil)
      places.answer(photo.url, with: solid(.blue))
      let gate = Latch()
      places.fetchLatch = gate
      places.latchedURL = photo.url
      let host = UIHostingController(rootView: MediaImage(photo).environment(\.mediaContext, context).environment(\.displayScale, 1).frame(width: 50, height: 50))
      let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 50, height: 50))
      window.rootViewController = host
      window.isHidden = false
      defer { gate.open(); window.isHidden = true }
      host.view.backgroundColor = .clear
      host.safeAreaRegions = []
      host.view.frame = window.bounds
      host.view.layoutIfNeeded()
      let first = centerPixel(of: host.view)
      #expect(first[0] > 200 && first[2] < 60)
      gate.open()
      await until("the requested size replaces the interim image") { let pixel = centerPixel(of: host.view); return pixel[2] > 200 && pixel[0] < 60 }
    }

    @Test
    func blurredBackdropSharesTheLoadedImage() async {
      let places = RecordingPlaces()
      ImageMemory.install(ImageCache(), on: places)
      let context = MediaContext(places: places)
      let photo = Photo(url: "https://o.test/backdrop-\(UUID().uuidString).png")
      places.answer(photo.url, with: solid(.blue))
      let host = UIHostingController(rootView: MediaImage(photo, contentMode: .fit, backdrop: .blurred(radius: 8)).environment(\.mediaContext, context).frame(width: 32, height: 32))
      let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 32, height: 32))
      window.rootViewController = host
      window.isHidden = false
      defer { window.isHidden = true }
      host.safeAreaRegions = []
      host.view.frame = window.bounds
      await until("backdrop and decode are present") { backdropIdentifiers(host.view).contains("media-image.backdrop") && places.imageStats().snapshot().decodes == 1 }
      #expect(places.fetchedURLs == [photo.url])
      #expect(places.imageStats().snapshot().requests == 1)
      #expect(places.imageStats().snapshot().decodes == 1)
      host.rootView = MediaImage(photo, contentMode: .fit).environment(\.mediaContext, context).frame(width: 32, height: 32)
      await until("backdrop is removed") { !backdropIdentifiers(host.view).contains("media-image.backdrop") }
    }

    @Test
    func inv_image_48_the_preview_shows_until_the_first_frame_of_the_main_image_and_takes_no_place_of_its_own() async {
      let places = RecordingPlaces()
      ImageMemory.install(ImageCache(), on: places)
      let context = MediaContext(places: places)
      let main = Photo(url: "https://o.test/uploaded-\(UUID().uuidString).png")
      let file = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).png")
      try? solid(.red).write(to: file)
      defer { try? FileManager.default.removeItem(at: file) }
      let local = Photo(url: file.absoluteString)
      places.answer(main.url, with: solid(.blue))
      places.answer(local.url, with: solid(.red))
      let gate = Latch()
      places.fetchLatch = gate
      places.latchedURL = main.url
      let host = UIHostingController(rootView: MediaImage(main, preview: local).environment(\.mediaContext, context).frame(width: 32, height: 32))
      let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 32, height: 32))
      window.rootViewController = host
      window.isHidden = false
      defer { window.isHidden = true }
      host.view.backgroundColor = .clear
      host.safeAreaRegions = []
      host.view.frame = window.bounds

      await until("the preview is on screen") { let pixel = centerPixel(of: host.view); return pixel[0] > 200 && pixel[2] < 60 }
      #expect(places.visibleCalls(of: local.url).isEmpty)

      gate.open()
      await until("the main image replaces the preview") { let pixel = centerPixel(of: host.view); return pixel[2] > 200 && pixel[0] < 60 }
      #expect(places.visibleCalls(of: local.url).isEmpty)
      #expect(places.visibleCalls(of: main.url) == [true])
      #expect(!places.mainThreadCalls.contains("cacheKey"))
    }

    @Test
    func inv_placeholder_13_a_source_switched_from_a_local_file_to_a_remote_url_with_that_file_as_preview_never_draws_an_empty_frame() async {
      let places = RecordingPlaces()
      ImageMemory.install(ImageCache(), on: places)
      let context = MediaContext(places: places)
      let file = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).png")
      try? solid(.red).write(to: file)
      defer { try? FileManager.default.removeItem(at: file) }
      let local = Photo(url: file.absoluteString)
      let remote = Photo(url: "https://o.test/uploaded-\(UUID().uuidString).png")
      places.answer(local.url, with: solid(.red))
      places.answer(remote.url, with: solid(.blue))
      places.fetchLatch = Latch()
      places.latchedURL = remote.url
      places.slowKey(for: remote.url)
      let model = Switch(url: local.url)
      let host = UIHostingController(rootView: Switching(model: model, preview: local).environment(\.mediaContext, context).frame(width: 32, height: 32))
      let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 32, height: 32))
      window.rootViewController = host
      window.isHidden = false
      defer { window.isHidden = true }
      host.view.backgroundColor = .clear
      host.safeAreaRegions = []
      host.view.frame = window.bounds

      await until("the local file is drawn as the main image") { let pixel = centerPixel(of: host.view); return pixel[0] > 200 && pixel[2] < 60 }

      model.url = remote.url
      var blank = 0
      for _ in 0..<80 {
        RunLoop.main.run(until: Date().addingTimeInterval(0.005))
        await Task.yield()
        let pixel = centerPixel(of: host.view)
        if !(pixel[0] > 200 && pixel[2] < 60) { blank += 1 }
      }
      #expect(blank == 0)
    }
  }
}
#endif
