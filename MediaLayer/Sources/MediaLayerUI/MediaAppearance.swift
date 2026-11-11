import CoreGraphics
import SwiftUI

/// The outline of the layer that owns the media contents.
public enum MediaShape: Equatable, Sendable {
  case rectangle
  case circle
  case rounded(CGFloat)

  func radius(in size: CGSize) -> CGFloat {
    switch self {
    case .rectangle: 0
    case .circle: min(size.width, size.height) / 2
    case .rounded(let radius): max(radius, 0)
    }
  }
}

private struct MediaShapeKey: EnvironmentKey {
  static let defaultValue = MediaShape.rectangle
}

private struct MediaGrayscaleKey: EnvironmentKey {
  static let defaultValue = false
}

extension EnvironmentValues {
  var mediaShape: MediaShape {
    get { self[MediaShapeKey.self] }
    set { self[MediaShapeKey.self] = newValue }
  }

  var mediaGrayscale: Bool {
    get { self[MediaGrayscaleKey.self] }
    set { self[MediaGrayscaleKey.self] = newValue }
  }
}

extension View {
  /// Rounds the layer that holds the media contents of `MediaImage` and `MediaVideo` below. No SwiftUI mask is added.
  public func mediaShape(_ shape: MediaShape) -> some View {
    environment(\.mediaShape, shape)
  }

  /// Draws the `MediaImage` below in grayscale. Each decoded frame is converted once, off the main thread for animations.
  public func mediaGrayscale(_ on: Bool = true) -> some View {
    environment(\.mediaGrayscale, on)
  }
}

enum GrayscaleImage {
  static func convert(_ image: CGImage) -> CGImage {
    let space = CGColorSpaceCreateDeviceGray()
    guard let context = CGContext(data: nil, width: image.width, height: image.height, bitsPerComponent: 8, bytesPerRow: 0, space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return image }

    context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
    return context.makeImage() ?? image
  }
}
