import CoreGraphics
import SwiftUI
#if canImport(UIKit)
import QuartzCore
#endif

/// Where a fitted or filled image sits in its frame, and the template tint, for every draw path of `MediaImage`.
enum ImageAlign {
  static func unit(_ alignment: Alignment) -> CGPoint {
    let x: CGFloat = alignment.horizontal == .leading ? 0 : alignment.horizontal == .trailing ? 1 : 0.5
    let y: CGFloat = alignment.vertical == .top ? 0 : alignment.vertical == .bottom ? 1 : 0.5
    return CGPoint(x: x, y: y)
  }

  /// The part of the image, in unit coordinates, that a filled layer of `bounds` shows.
  static func fillRect(image: CGSize, bounds: CGSize, alignment: Alignment) -> CGRect {
    guard image.width > 0, image.height > 0, bounds.width > 0, bounds.height > 0 else { return CGRect(x: 0, y: 0, width: 1, height: 1) }

    let scale = max(bounds.width / image.width, bounds.height / image.height)
    let width = min(1, bounds.width / (image.width * scale))
    let height = min(1, bounds.height / (image.height * scale))
    let anchor = unit(alignment)
    return CGRect(x: (1 - width) * anchor.x, y: (1 - height) * anchor.y, width: width, height: height)
  }

  /// The size a fitted image takes inside `proposal`.
  static func fitted(image: CGSize, in proposal: CGSize) -> CGSize {
    guard image.width > 0, image.height > 0 else { return proposal }

    let scale = min(proposal.width / image.width, proposal.height / image.height)
    return CGSize(width: image.width * scale, height: image.height * scale)
  }

  #if canImport(UIKit)
  /// Layer geometry: a filled image shows the aligned part of the picture, a fitted one is sized by `size(for:)` and placed by SwiftUI.
  static func apply(to layer: CALayer, image: CGSize, bounds: CGSize, contentMode: ContentMode, alignment: Alignment) {
    if contentMode == .fill {
      layer.contentsGravity = .resize
      layer.contentsRect = fillRect(image: image, bounds: bounds, alignment: alignment)
    } else {
      layer.contentsGravity = .resizeAspect
      layer.contentsRect = CGRect(x: 0, y: 0, width: 1, height: 1)
    }
  }

  #endif

  static func size(for proposal: ProposedViewSize, image: CGSize, contentMode: ContentMode, alignment: Alignment) -> CGSize {
    let full = proposal.replacingUnspecifiedDimensions()
    guard contentMode == .fit, unit(alignment) != CGPoint(x: 0.5, y: 0.5) else { return full }

    return fitted(image: image, in: full)
  }
}

enum TintedImage {
  /// Keeps the alpha of `image` and replaces its color, as a template image does.
  static func convert(_ image: CGImage, tint: CGColor) -> CGImage {
    guard let context = CGContext(data: nil, width: image.width, height: image.height, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return image }

    let rect = CGRect(x: 0, y: 0, width: image.width, height: image.height)
    context.draw(image, in: rect)
    context.setBlendMode(.sourceIn)
    context.setFillColor(tint)
    context.fill(rect)
    return context.makeImage() ?? image
  }
}

struct ImagePlacement: Equatable {
  var image = CGSize.zero
  var contentMode = ContentMode.fill
  var alignment = Alignment.center
}

/// The SwiftUI draw path of a still: template tint, fit or fill, and the alignment of the overflow.
struct AlignedImage: View {
  let image: PlatformImage
  let contentMode: ContentMode
  let alignment: Alignment
  let tint: Color?
  let size: CGSize

  var body: some View {
    TemplateImage(image: image, contentMode: contentMode, tint: tint)
    .frame(width: size.width, height: size.height, alignment: alignment)
    .clipped()
  }
}

struct TemplateImage: View {
  let image: PlatformImage
  let contentMode: ContentMode
  let tint: Color?

  var body: some View {
    if let tint {
      Image(platform: image).renderingMode(.template).resizable().aspectRatio(contentMode: contentMode).foregroundStyle(tint)
    } else {
      Image(platform: image).resizable().aspectRatio(contentMode: contentMode)
    }
  }
}
