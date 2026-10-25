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

extension GlobalMedia {
  @MainActor
  @Suite(.timeLimit(.minutes(3)))
  struct MediaImagePreviewTests {
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
    }
  }
}
#endif
