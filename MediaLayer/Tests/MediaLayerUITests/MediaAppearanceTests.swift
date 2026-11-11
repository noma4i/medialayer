#if canImport(UIKit)
import AVFoundation
import SwiftUI
import Testing
import UIKit
@testable import MediaLayerUI

private func redImage() throws -> CGImage {
  let context = try #require(CGContext(
    data: nil, width: 16, height: 16, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
  ))
  context.setFillColor(CGColor(red: 1, green: 0, blue: 0, alpha: 1))
  context.fill(CGRect(x: 0, y: 0, width: 16, height: 16))
  return try #require(context.makeImage())
}

@MainActor
private func stillHost(in view: UIView) -> StillLayerHost? {
  if let found = view as? StillLayerHost { return found }
  for child in view.subviews { if let found = stillHost(in: child) { return found } }
  return nil
}

enum ShapedLayer: String, CaseIterable, CustomTestStringConvertible {
  case animatedFrames, still, videoSurface

  var testDescription: String { rawValue }
}

struct ShapeCase: Sendable, CustomTestStringConvertible {
  let shape: MediaShape
  let radius: CGFloat

  var testDescription: String { "\(shape)" }

  static let size = CGSize(width: 40, height: 20)
  static let all = [
    ShapeCase(shape: .circle, radius: 10),
    ShapeCase(shape: .rounded(6), radius: 6),
    ShapeCase(shape: .rectangle, radius: 0)
  ]
}

@MainActor
@Suite(.timeLimit(.minutes(1)))
struct MediaAppearanceTests {
  // mutation: GrayscaleImage.convert returns its input or draws into an RGB context
  @Test
  func grayscale_conversion_returns_a_monochrome_image() throws {
    let converted = GrayscaleImage.convert(try redImage())
    #expect(converted.colorSpace?.model == .monochrome)
  }

  // mutation: StillLayerView sets layer.contents to the source image when grayscale is on
  @Test(arguments: [false, true])
  func a_still_image_shows_monochrome_contents_on_its_layer_only_in_grayscale(grayscale: Bool) async throws {
    let source = try redImage()
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 100, height: 100))
    let host = UIHostingController(rootView: StillLayerView(image: UIImage(cgImage: source), contentMode: .fit, grayscale: grayscale, shape: .rectangle).frame(width: 40, height: 20))
    window.rootViewController = host
    window.isHidden = false
    defer { window.isHidden = true; window.rootViewController = nil }

    await eventually("the still image reached its layer") { stillHost(in: host.view)?.layer.contents != nil }
    let contents = try #require(stillHost(in: host.view)?.layer.contents)
    let shown = contents as! CGImage
    let expected: CGColorSpaceModel? = grayscale ? .monochrome : source.colorSpace?.model
    #expect(shown.colorSpace?.model == expected)
  }

  // mutation: MediaShape.radius maps a shape to a wrong radius, a host skips cornerRadius in layout, or a rounded layer does not mask its bounds
  @Test(arguments: ShapedLayer.allCases, ShapeCase.all)
  func a_shape_rounds_the_layer_that_owns_the_media_contents(_ kind: ShapedLayer, _ shape: ShapeCase) throws {
    let frame = CGRect(origin: .zero, size: ShapeCase.size)
    let rounded: CALayer
    let owner: UIView
    switch kind {
    case .animatedFrames:
      let view = FrameLayerView(frame: frame)
      let frames = try decodedFrames()
      AnimatedFramesView(frames: frames, contentMode: .fill, isPlaying: false, playbackRate: 1, shape: shape.shape).update(view, playback: AnimatedFramesView.Playback())
      owner = view
      rounded = view.layer
    case .still:
      let view = StillLayerHost(frame: frame)
      view.shape = shape.shape
      owner = view
      rounded = view.layer
    case .videoSurface:
      let view = SurfaceView(frame: frame)
      let host = SurfaceHost(places: RecordingPlaces())
      let player = AVPlayerLayer()
      host.layer.addSublayer(player)
      view.host = host
      view.layer.addSublayer(host.layer)
      view.shape = shape.shape
      owner = view
      rounded = player
    }
    owner.setNeedsLayout()
    owner.layoutIfNeeded()

    #expect(rounded.cornerRadius == shape.radius)
    if shape.radius > 0 { #expect(rounded.masksToBounds) }
  }
}
#endif
