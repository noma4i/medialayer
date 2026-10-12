import CoreGraphics
import Foundation
import ImageIO
import MediaLayer
import Nuke
import Testing
@testable import MediaLayerUI

private struct AlphaSource: MediaSource {
  let url: String
  var kind: Kind { .image }
  var headers: [String: String] = [:]
  var size: MediaSize?
}

private let side = 64

private func fixture(_ name: String) throws -> Data {
  try Data(contentsOf: #require(Bundle.module.url(forResource: name, withExtension: nil, subdirectory: "Fixtures/alpha")))
}

private func overBlue(_ image: CGImage) -> [UInt8] {
  var pixels = [UInt8](repeating: 0, count: side * side * 4)
  pixels.withUnsafeMutableBytes { bytes in
    let context = CGContext(
      data: bytes.baseAddress, width: side, height: side, bitsPerComponent: 8, bytesPerRow: side * 4,
      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    )!
    context.setFillColor(CGColor(colorSpace: CGColorSpaceCreateDeviceRGB(), components: [0, 0, 1, 1])!)
    context.fill(CGRect(x: 0, y: 0, width: side, height: side))
    context.draw(image, in: CGRect(x: 0, y: 0, width: side, height: side))
  }
  return pixels
}

private func reference(_ data: Data, index: Int) -> [UInt8] {
  let source = CGImageSourceCreateWithData(data as CFData, nil)!
  let options: [CFString: Any] = [kCGImageSourceCreateThumbnailFromImageAlways: true, kCGImageSourceThumbnailMaxPixelSize: side]
  return overBlue(CGImageSourceCreateThumbnailAtIndex(source, index, options as CFDictionary)!)
}

private func worstChannelError(_ shown: [UInt8], _ expected: [UInt8]) -> Int {
  zip(shown, expected).map { abs(Int($0) - Int($1)) }.max() ?? 0
}

private func loadedImage(_ data: Data) async throws -> CGImage {
  let places = RecordingPlaces()
  places.imageResult = .success(data)
  ImageMemory.install(ImageCache(), on: places)
  let request = try #require(MediaImagePipeline.request(for: AlphaSource(url: "https://o.test/a"), points: CGSize(width: side, height: side), scale: 1, contentMode: .fit))
  let image = try await MediaImagePipeline.make(places: places).image(for: request)
  #if canImport(UIKit)
  return try #require(image.cgImage)
  #else
  return try #require(image.cgImage(forProposedRect: nil, context: nil, hints: nil))
  #endif
}

extension GlobalMedia {
  @Suite(.timeLimit(.minutes(1)))
  struct AlphaCompositingTests {
    @Test(arguments: ["sticker.png", "sticker.webp", "sticker-lossy.webp", "sticker.gif"])
    func a_still_image_composites_over_the_background_without_a_backing_or_a_fringe(name: String) async throws {
      let data = try fixture(name)
      let shown = overBlue(try await loadedImage(data))
      let expected = reference(data, index: 0)

      let corner = Array(shown[0 ..< 4])
      let worst = worstChannelError(shown, expected)
      #expect(corner == [0, 0, 255, 255], "corner \(corner) ref \(Array(expected[0 ..< 4]))")
      #expect(worst <= 3, "worst \(worst)")
    }

    @Test(arguments: ["sticker.gif", "sticker.apng", "sticker-anim.webp"])
    func every_frame_of_an_animation_composites_without_a_backing_or_a_fringe(name: String) throws {
      let data = try fixture(name)
      let frames = try #require(AnimatedFrames.decode(data, maxPixel: side))

      #expect(frames.frameCount == 3)
      for (index, image) in (0 ..< frames.frameCount).compactMap(frames.image(at:)).enumerated() {
        let shown = overBlue(image)
        #expect(Array(shown[0 ..< 4]) == [0, 0, 255, 255])
        #expect(worstChannelError(shown, reference(data, index: index)) <= 3)
      }
    }
  }
}
