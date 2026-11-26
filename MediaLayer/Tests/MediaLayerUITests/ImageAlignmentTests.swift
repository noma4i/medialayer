#if canImport(UIKit)
import SwiftUI
import Testing
import UIKit
@testable import MediaLayerUI

private func twoColorImage() throws -> UIImage {
  let context = try #require(CGContext(
    data: nil, width: 16, height: 16, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
  ))
  context.setFillColor(CGColor(red: 0, green: 0, blue: 1, alpha: 1))
  context.fill(CGRect(x: 0, y: 0, width: 16, height: 8))
  context.setFillColor(CGColor(red: 1, green: 0, blue: 0, alpha: 1))
  context.fill(CGRect(x: 0, y: 8, width: 16, height: 8))
  return UIImage(cgImage: try #require(context.makeImage()))
}

private func pixel(_ image: CGImage, x: Int, y: Int) -> (r: Int, g: Int, b: Int, a: Int) {
  var data = [UInt8](repeating: 0, count: 4)
  let context = CGContext(data: &data, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
  context.draw(image, in: CGRect(x: -x, y: -(image.height - 1 - y), width: image.width, height: image.height))
  return (Int(data[0]), Int(data[1]), Int(data[2]), Int(data[3]))
}

@MainActor
@Suite(.timeLimit(.minutes(1)))
struct ImageAlignmentTests {
  static let size = CGSize(width: 40, height: 10)

  // mutation: AlignedImage or StillLayerView drops the alignment, so a filled wide frame shows the middle of the picture
  @Test(arguments: [false, true])
  func inv_image_120_a_filled_image_aligned_top_keeps_its_top_edge_at_the_frame_top(layer: Bool) throws {
    let image = try twoColorImage()
    let shown: AnyView = layer
      ? AnyView(StillLayerView(image: image, contentMode: .fill, grayscale: false, shape: .rectangle, alignment: .top).frame(width: Self.size.width, height: Self.size.height))
      : AnyView(AlignedImage(image: image, contentMode: .fill, alignment: .top, tint: nil, size: Self.size))
    let frame = try #require(firstFrame(of: shown, size: Self.size))
    let top = pixel(frame, x: 20, y: 0)
    let bottom = pixel(frame, x: 20, y: 9)
    #expect(top.r > 200 && top.b < 50)
    #expect(bottom.r > 200 && bottom.b < 50)
  }

  // mutation: the tint is ignored, so opaque pixels keep their own color
  @Test(arguments: [false, true])
  func inv_image_120_a_tint_replaces_the_color_and_keeps_the_alpha(layer: Bool) throws {
    let image = try twoColorImage()
    let side = CGSize(width: 16, height: 16)
    let shown: AnyView = layer
      ? AnyView(StillLayerView(image: image, contentMode: .fit, grayscale: false, shape: .rectangle, tint: UIColor.green.cgColor).frame(width: side.width, height: side.height))
      : AnyView(AlignedImage(image: image, contentMode: .fit, alignment: .center, tint: Color(red: 0, green: 1, blue: 0), size: side))
    let frame = try #require(firstFrame(of: shown, size: side))
    let top = pixel(frame, x: 8, y: 2)
    let bottom = pixel(frame, x: 8, y: 13)
    #expect(top.g > 200 && top.r < 50 && top.b < 50)
    #expect(bottom.g > 200 && bottom.r < 50 && bottom.b < 50)
  }
}
#endif
