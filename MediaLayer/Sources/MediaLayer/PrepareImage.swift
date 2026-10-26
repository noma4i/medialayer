import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

extension MediaLayer {
  /// A photo made upright by its EXIF orientation and saved as JPEG. No metadata is copied, so the editor tag and every EXIF field are gone.
  public func prepareImage(_ source: some MediaSource, quality: Double = 0.9) async throws -> PreparedMedia {
    let file = try preparableFile(source.url)
    let work = Task.detached {
      try await ImagePreparation.run(file, quality: quality) { bytes in
        let hold = try await self.reserveDecode(bytes: bytes)
        return { hold.release() }
      }
    }
    return try await withTaskCancellationHandler { try await work.value } onCancel: { work.cancel() }
  }
}

enum ImagePreparation {
  typealias Admit = @Sendable (UInt64) async throws -> @Sendable () -> Void

  static func run(_ file: URL, quality: Double, admit: Admit) async throws -> PreparedMedia {
    guard let source = CGImageSourceCreateWithURL(file as CFURL, nil),
          let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
          let width = properties[kCGImagePropertyPixelWidth] as? Int,
          let height = properties[kCGImagePropertyPixelHeight] as? Int
    else { throw MediaError.Fetch(detail: "\(file.path) is not an image") }
    guard let cost = decodeCost(width: UInt32(clamping: width), height: UInt32(clamping: height)) else {
      throw MediaError.Fetch(detail: "\(file.path) is above the pixel limit")
    }
    let release = try await admit(cost)
    defer { release() }
    guard let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else { throw MediaError.Fetch(detail: "\(file.path) is not an image") }
    try Task.checkCancellation()
    let tag = (properties[kCGImagePropertyOrientation] as? UInt32) ?? 1
    let upright = try draw(image, turn: photoTurn(exifOrientation: tag))
    try Task.checkCancellation()
    let output = preparedOutput(extension: "jpg")
    guard let destination = CGImageDestinationCreateWithURL(output as CFURL, UTType.jpeg.identifier as CFString, 1, nil) else {
      throw MediaError.Unexpected(detail: "the JPEG destination cannot be created")
    }
    CGImageDestinationAddImage(destination, upright, [kCGImageDestinationLossyCompressionQuality: jpegQuality(quality: quality)] as CFDictionary)
    guard CGImageDestinationFinalize(destination) else {
      try? FileManager.default.removeItem(at: output)
      throw MediaError.Unexpected(detail: "the JPEG cannot be written")
    }
    return PreparedMedia(url: output, isNew: true)
  }

  private static func draw(_ image: CGImage, turn: PhotoTurn) throws -> CGImage {
    let width = CGFloat(image.width)
    let height = CGFloat(image.height)
    let sideways = turn.rotationDegrees % 180 == 90
    let outWidth = sideways ? image.height : image.width
    let outHeight = sideways ? image.width : image.height
    guard let context = CGContext(
      data: nil, width: outWidth, height: outHeight, bitsPerComponent: 8, bytesPerRow: 0,
      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
    ) else { throw MediaError.Unexpected(detail: "the photo canvas cannot be created") }
    var turned: CGAffineTransform
    switch turn.rotationDegrees {
    case 90: turned = CGAffineTransform(a: 0, b: 1, c: -1, d: 0, tx: height, ty: 0)
    case 180: turned = CGAffineTransform(a: -1, b: 0, c: 0, d: -1, tx: width, ty: height)
    case 270: turned = CGAffineTransform(a: 0, b: -1, c: 1, d: 0, tx: 0, ty: width)
    default: turned = .identity
    }
    if turn.flipHorizontal { turned = turned.concatenating(CGAffineTransform(a: -1, b: 0, c: 0, d: 1, tx: CGFloat(outWidth), ty: 0)) }
    context.translateBy(x: 0, y: CGFloat(outHeight))
    context.scaleBy(x: 1, y: -1)
    context.concatenate(turned)
    context.translateBy(x: 0, y: height)
    context.scaleBy(x: 1, y: -1)
    context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
    guard let result = context.makeImage() else { throw MediaError.Unexpected(detail: "the photo cannot be turned") }
    return result
  }
}
