import CoreGraphics
import Foundation
import ImageIO
import MediaLayer
import UniformTypeIdentifiers

enum PrepareImageWrite {
  static func run(source sourceURL: URL, output: URL, plan: PhotoPlan, quality: Double, format: ImageFormat) throws {
    guard let source = CGImageSourceCreateWithURL(sourceURL as CFURL, nil) else {
      throw PrepareFailure(message: "\(sourceURL.path) is not an image")
    }
    let options: [CFString: Any] = [
      kCGImageSourceCreateThumbnailFromImageAlways: true,
      kCGImageSourceCreateThumbnailWithTransform: false,
      kCGImageSourceShouldCacheImmediately: true,
      kCGImageSourceThumbnailMaxPixelSize: plan.thumbnailSide
    ]
    guard var result = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else {
      throw PrepareFailure(message: "\(sourceURL.path) is not an image")
    }
    let flatten = format != .png
    var rewritten = false
    if plan.turn.rotationDegrees != 0 || plan.turn.flipHorizontal {
      result = try turned(result, plan.turn, flatten: flatten)
      rewritten = true
    }
    try Task.checkCancellation()
    if plan.needsDraw {
      result = try drawn(result, plan: plan, flatten: flatten)
      rewritten = true
    }
    if flatten && !rewritten && result.hasAlpha { result = try flattened(result) }
    try Task.checkCancellation()
    let type = format == .png ? UTType.png : UTType.jpeg
    guard let destination = CGImageDestinationCreateWithURL(output as CFURL, type.identifier as CFString, 1, nil) else {
      throw PrepareFailure(message: "the image destination cannot be created")
    }
    let encoding: CFDictionary? = format == .png ? nil : [kCGImageDestinationLossyCompressionQuality: quality] as CFDictionary
    CGImageDestinationAddImage(destination, result, encoding)
    guard CGImageDestinationFinalize(destination) else { throw PrepareFailure(message: "the image cannot be encoded") }
  }

  private static func drawn(_ image: CGImage, plan: PhotoPlan, flatten: Bool) throws -> CGImage {
    let across = Double(image.width) / Double(plan.upright.width)
    let down = Double(image.height) / Double(plan.upright.height)
    let left = min(max(Int((Double(plan.crop.x) * across).rounded()), 0), image.width - 1)
    let top = min(max(Int((Double(plan.crop.y) * down).rounded()), 0), image.height - 1)
    let right = min(max(Int((Double(plan.crop.x + plan.crop.width) * across).rounded()), left + 1), image.width)
    let bottom = min(max(Int((Double(plan.crop.y + plan.crop.height) * down).rounded()), top + 1), image.height)
    guard let cropped = image.cropping(to: CGRect(x: left, y: top, width: right - left, height: bottom - top)) else {
      throw PrepareFailure(message: "the photo cannot be cropped")
    }
    let context = try canvas(width: Int(plan.output.width), height: Int(plan.output.height), onWhite: flatten)
    context.interpolationQuality = .high
    if plan.mirror {
      context.translateBy(x: CGFloat(plan.output.width), y: 0)
      context.scaleBy(x: -1, y: 1)
    }
    context.draw(cropped, in: CGRect(x: 0, y: 0, width: Int(plan.output.width), height: Int(plan.output.height)))
    guard let result = context.makeImage() else { throw PrepareFailure(message: "the photo cannot be drawn") }
    return result
  }

  private static func turned(_ image: CGImage, _ turn: PhotoTurn, flatten: Bool) throws -> CGImage {
    let width = CGFloat(image.width)
    let height = CGFloat(image.height)
    let sideways = turn.rotationDegrees % 180 == 90
    let outWidth = sideways ? image.height : image.width
    let outHeight = sideways ? image.width : image.height
    let context = try canvas(width: outWidth, height: outHeight, onWhite: flatten)
    var matrix: CGAffineTransform
    switch turn.rotationDegrees {
    case 90: matrix = CGAffineTransform(a: 0, b: 1, c: -1, d: 0, tx: height, ty: 0)
    case 180: matrix = CGAffineTransform(a: -1, b: 0, c: 0, d: -1, tx: width, ty: height)
    case 270: matrix = CGAffineTransform(a: 0, b: -1, c: 1, d: 0, tx: 0, ty: width)
    default: matrix = .identity
    }
    if turn.flipHorizontal { matrix = matrix.concatenating(CGAffineTransform(a: -1, b: 0, c: 0, d: 1, tx: CGFloat(outWidth), ty: 0)) }
    context.translateBy(x: 0, y: CGFloat(outHeight))
    context.scaleBy(x: 1, y: -1)
    context.concatenate(matrix)
    context.translateBy(x: 0, y: height)
    context.scaleBy(x: 1, y: -1)
    context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
    guard let result = context.makeImage() else { throw PrepareFailure(message: "the photo cannot be turned") }
    return result
  }

  /// An opaque canvas; a JPEG has no alpha, so its canvas starts white like `encodeImage` and the Android writer.
  private static func canvas(width: Int, height: Int, onWhite: Bool) throws -> CGContext {
    guard let context = CGContext(
      data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
    ) else { throw PrepareFailure(message: "the photo canvas cannot be created") }
    if onWhite {
      context.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 1))
      context.fill(CGRect(x: 0, y: 0, width: width, height: height))
    }
    return context
  }

  private static func flattened(_ image: CGImage) throws -> CGImage {
    let context = try canvas(width: image.width, height: image.height, onWhite: true)
    context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
    guard let result = context.makeImage() else { throw PrepareFailure(message: "the photo cannot be flattened") }
    return result
  }
}

private extension CGImage {
  var hasAlpha: Bool {
    switch alphaInfo {
    case .none, .noneSkipFirst, .noneSkipLast: false
    default: true
    }
  }
}
