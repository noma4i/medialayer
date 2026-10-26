import CoreGraphics
import Foundation
import ImageIO
import MediaLayer
#if canImport(UIKit)
import UIKit
#else
import AppKit
#endif

typealias DecodeAdmit = @Sendable (UInt64) async throws -> @Sendable () -> Void

extension MediaLayer {
  var decodeAdmit: DecodeAdmit {
    { bytes in
      let hold = try await self.reserveDecode(bytes: bytes)
      return { hold.release() }
    }
  }
}

/// A local picture as an upright image: the EXIF turn is applied, the long side is cut to `maxSide` (nil keeps the size), a picture is never enlarged.
/// The decode runs off the main thread and takes its raster from the shared memory budget of the open layer, a refusal is a `MediaError`.
public enum MediaImageDecode {
  public static func decode(file: URL, maxSide: Int? = nil) async throws -> PlatformImage {
    let admit = try openedAdmit()
    return try await decode(maxSide: maxSide, circle: false, admit: admit) { CGImageSourceCreateWithURL(file as CFURL, nil) }
  }

  public static func decode(data: Data, maxSide: Int? = nil) async throws -> PlatformImage {
    let admit = try openedAdmit()
    return try await decode(maxSide: maxSide, circle: false, admit: admit) { CGImageSourceCreateWithData(data as CFData, nil) }
  }

  /// The size of an image in memory as `decode` would give it (the EXIF turn applied), read from the header without decoding the pixels. `nil` when the bytes are no image.
  public static func imageSize(data: Data) -> ImageSize? {
    guard let source = CGImageSourceCreateWithData(data as CFData, nil),
          let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
          let width = properties[kCGImagePropertyPixelWidth] as? Int,
          let height = properties[kCGImagePropertyPixelHeight] as? Int,
          width > 0, height > 0
    else { return nil }
    let sideways = photoTurn(exifOrientation: (properties[kCGImagePropertyOrientation] as? UInt32) ?? 1).rotationDegrees % 180 == 90
    return ImageSize(width: UInt32(clamping: sideways ? height : width), height: UInt32(clamping: sideways ? width : height))
  }

  private static func openedAdmit() throws -> DecodeAdmit {
    guard let layer = Media.current as? MediaLayer else { throw MediaError.Closed }
    return layer.decodeAdmit
  }

  static func decode(maxSide: Int?, circle: Bool, admit: @escaping DecodeAdmit, makeSource: @escaping @Sendable () -> CGImageSource?) async throws -> PlatformImage {
    let work = Task.detached { try await run(makeSource(), maxSide: maxSide, circle: circle, admit: admit) }
    return try await withTaskCancellationHandler { try await work.value } onCancel: { work.cancel() }
  }

  private static func run(_ source: CGImageSource?, maxSide: Int?, circle: Bool, admit: DecodeAdmit) async throws -> PlatformImage {
    guard let source,
          let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
          let width = properties[kCGImagePropertyPixelWidth] as? Int,
          let height = properties[kCGImagePropertyPixelHeight] as? Int
    else { throw MediaError.Fetch(detail: "the source is not an image") }
    let tag = (properties[kCGImagePropertyOrientation] as? UInt32) ?? 1
    guard let plan = imageDecodePlan(width: UInt32(clamping: width), height: UInt32(clamping: height), exifOrientation: tag, maxSide: maxSide.map { UInt32(clamping: max($0, 0)) }) else {
      throw MediaError.Fetch(detail: "the image is above the pixel limit")
    }
    let release = try await admit(plan.cost)
    defer { release() }
    try Task.checkCancellation()
    let options: [CFString: Any] = [
      kCGImageSourceCreateThumbnailFromImageAlways: true,
      kCGImageSourceCreateThumbnailWithTransform: true,
      kCGImageSourceShouldCacheImmediately: true,
      kCGImageSourceThumbnailMaxPixelSize: max(plan.decodeWidth, plan.decodeHeight)
    ]
    guard let upright = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { throw MediaError.Fetch(detail: "the source is not an image") }
    try Task.checkCancellation()
    let result = try circle ? roundSquare(upright, side: max(maxSide ?? 1, 1)) : upright
    return platformImage(result)
  }

  private static func roundSquare(_ image: CGImage, side: Int) throws -> CGImage {
    guard let context = CGContext(
      data: nil, width: side, height: side, bitsPerComponent: 8, bytesPerRow: 0,
      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ) else { throw MediaError.Unexpected(detail: "the circle canvas cannot be created") }
    let scale = CGFloat(side) / CGFloat(min(image.width, image.height))
    let drawn = CGSize(width: CGFloat(image.width) * scale, height: CGFloat(image.height) * scale)
    context.addEllipse(in: CGRect(x: 0, y: 0, width: side, height: side))
    context.clip()
    context.interpolationQuality = .high
    context.draw(image, in: CGRect(x: (CGFloat(side) - drawn.width) / 2, y: (CGFloat(side) - drawn.height) / 2, width: drawn.width, height: drawn.height))
    guard let result = context.makeImage() else { throw MediaError.Unexpected(detail: "the circle cannot be drawn") }
    return result
  }

  private static func platformImage(_ image: CGImage) -> PlatformImage {
    #if canImport(UIKit)
    UIImage(cgImage: image)
    #else
    NSImage(cgImage: image, size: NSSize(width: image.width, height: image.height))
    #endif
  }
}
