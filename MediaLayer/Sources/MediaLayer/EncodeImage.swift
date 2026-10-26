import CoreGraphics
import Foundation
import ImageIO

extension MediaLayer {
  /// An image held in memory saved as a JPEG or PNG file. `exifOrientation` (1 to 8) turns the image upright first, a JPEG has no transparency so it is flattened onto white.
  /// `quality` (0 to 1) is for JPEG only. The file belongs to the caller.
  public func encodeImage(_ image: CGImage, exifOrientation: UInt32 = 1, format: ImageFormat = .jpeg, quality: Double = 0.9) async throws -> PreparedMedia {
    let work = Task.detached {
      let data = try await ImageEncoding.run(image, exifOrientation: exifOrientation, format: format, quality: quality, admit: self.encodeAdmit)
      let output = preparedOutput(extension: format == .png ? "png" : "jpg")
      do {
        try data.write(to: output)
      } catch {
        try? FileManager.default.removeItem(at: output)
        throw MediaError.Unexpected(detail: "the image cannot be written")
      }
      return PreparedMedia(url: output, isNew: true)
    }
    return try await withTaskCancellationHandler { try await work.value } onCancel: { work.cancel() }
  }

  /// The bytes of an image (any format ImageIO reads, such as WebP) encoded again as JPEG or PNG and turned upright by their EXIF orientation.
  /// The decoded raster is asked of the memory budget before the bytes are decoded.
  public func encodeImage(bytes: Data, format: ImageFormat = .jpeg, quality: Double = 0.9) async throws -> Data {
    let work = Task.detached {
      guard let source = CGImageSourceCreateWithData(bytes as CFData, nil),
            let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
            let width = properties[kCGImagePropertyPixelWidth] as? Int,
            let height = properties[kCGImagePropertyPixelHeight] as? Int
      else { throw MediaError.Fetch(detail: "the bytes are not an image") }
      let tag = (properties[kCGImagePropertyOrientation] as? UInt32) ?? 1
      let alpha = (properties[kCGImagePropertyHasAlpha] as? Bool) ?? false
      return try await ImageEncoding.run(
        width: UInt32(clamping: width), height: UInt32(clamping: height), exifOrientation: tag, hasAlpha: alpha, format: format, quality: quality, admit: self.encodeAdmit
      ) {
        guard let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else { throw MediaError.Fetch(detail: "the bytes are not an image") }
        return image
      }
    }
    return try await withTaskCancellationHandler { try await work.value } onCancel: { work.cancel() }
  }

  fileprivate var encodeAdmit: ImagePreparation.Admit {
    { bytes in
      let hold = try await self.reserveDecode(bytes: bytes)
      return { hold.release() }
    }
  }
}

enum ImageEncoding {
  static func run(_ image: CGImage, exifOrientation: UInt32, format: ImageFormat, quality: Double, admit: ImagePreparation.Admit) async throws -> Data {
    let alpha: Bool
    switch image.alphaInfo {
    case .none, .noneSkipFirst, .noneSkipLast: alpha = false
    default: alpha = true
    }
    return try await run(
      width: UInt32(clamping: image.width), height: UInt32(clamping: image.height), exifOrientation: exifOrientation,
      hasAlpha: alpha, format: format, quality: quality, admit: admit
    ) { image }
  }

  /// The memory is asked of the budget first, `makeImage` runs after the admission.
  static func run(
    width: UInt32, height: UInt32, exifOrientation: UInt32, hasAlpha: Bool, format: ImageFormat, quality: Double,
    admit: ImagePreparation.Admit, makeImage: () throws -> CGImage
  ) async throws -> Data {
    guard let plan = imageEncodePlan(width: width, height: height, exifOrientation: exifOrientation, hasAlpha: hasAlpha, format: format) else {
      throw MediaError.Fetch(detail: "the image has no pixel or is above the pixel limit")
    }
    let release = try await admit(plan.cost)
    defer { release() }
    try Task.checkCancellation()
    var result = try makeImage()
    if plan.turn.rotationDegrees != 0 || plan.turn.flipHorizontal { result = try ImagePreparation.draw(result, turn: plan.turn) }
    if plan.flatten { result = try onWhite(result) }
    try Task.checkCancellation()
    return try ImagePreparation.encoded(result, format: format, quality: quality)
  }

  private static func onWhite(_ image: CGImage) throws -> CGImage {
    guard let context = CGContext(
      data: nil, width: image.width, height: image.height, bitsPerComponent: 8, bytesPerRow: 0,
      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
    ) else { throw MediaError.Unexpected(detail: "the image canvas cannot be created") }
    context.setFillColor(CGColor(red: 1, green: 1, blue: 1, alpha: 1))
    context.fill(CGRect(x: 0, y: 0, width: image.width, height: image.height))
    context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
    guard let result = context.makeImage() else { throw MediaError.Unexpected(detail: "the image cannot be flattened") }
    return result
  }
}
