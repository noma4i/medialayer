import CoreGraphics
import Foundation
import ImageIO

extension MediaLayer {
  /// An image held in memory saved as a JPEG or PNG file. `exifOrientation` (1 to 8) turns the image upright first, a JPEG has no transparency so it is flattened onto white.
  /// `quality` (0 to 1) is for JPEG only. The file belongs to the caller.
  public func encodeImage(_ image: CGImage, exifOrientation: UInt32 = 1, format: ImageFormat = .jpeg, quality: Double = 0.9) async throws -> PreparedMedia {
    let work = Task.detached {
      let output = preparedOutput(extension: format == .png ? "png" : "jpg")
      do {
        try await ImageEncoding.run(image, exifOrientation: exifOrientation, format: format, quality: quality, admit: self.encodeAdmit) {
          try ImagePreparation.encode($0, format: format, quality: quality, to: output)
        }
      } catch {
        NativeFailures.removeFile(output)
        throw error
      }
      return PreparedMedia(url: output, isNew: true)
    }
    return try await withTaskCancellationHandler { try await work.value } onCancel: { work.cancel() }
  }

  /// The bytes of an image (any format ImageIO reads, such as WebP) encoded again as JPEG or PNG and turned upright by their EXIF orientation.
  /// The decoded raster is asked of the memory budget before the bytes are decoded.
  public func encodeImage(bytes: Data, format: ImageFormat = .jpeg, quality: Double = 0.9) async throws -> Data {
    let admit = encodeAdmit
    let work = Task.detached { try await ImageEncoding.run(bytes: bytes, format: format, quality: quality, admit: admit) }
    return try await withTaskCancellationHandler { try await work.value } onCancel: { work.cancel() }
  }

  fileprivate var encodeAdmit: ImagePreparation.Admit {
    { bytes in
      let hold = try await self.reserveDecode(bytes: bytes)
      return { hold.release() }
    }
  }
}

/// The bytes of an image encoded again as JPEG or PNG without an open layer: no proxy, no cache, one call. The raster the encode needs is checked
/// against `limitBytes` (the input limit of the library when `nil`) before the bytes are decoded, a size above it is refused with `MediaError.Overloaded`.
public enum MediaImageEncode {
  public static func encode(bytes: Data, format: ImageFormat = .jpeg, quality: Double = 0.9, limitBytes: UInt64? = nil) async throws -> Data {
    let work = Task.detached {
      try await ImageEncoding.run(bytes: bytes, format: format, quality: quality) { cost in
        try imageEncodeOutsideAdmit(cost: cost, limitBytes: limitBytes)
        return {}
      }
    }
    return try await withTaskCancellationHandler { try await work.value } onCancel: { work.cancel() }
  }
}

enum ImageEncoding {
  static func run(bytes: Data, format: ImageFormat, quality: Double, admit: ImagePreparation.Admit) async throws -> Data {
    guard let source = CGImageSourceCreateWithData(bytes as CFData, nil),
          let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
          let width = properties[kCGImagePropertyPixelWidth] as? Int,
          let height = properties[kCGImagePropertyPixelHeight] as? Int
    else { throw MediaError.Fetch(detail: "the bytes are not an image") }
    let tag = (properties[kCGImagePropertyOrientation] as? UInt32) ?? 1
    let alpha = (properties[kCGImagePropertyHasAlpha] as? Bool) ?? false
    return try await run(
      width: UInt32(clamping: width), height: UInt32(clamping: height), exifOrientation: tag, hasAlpha: alpha, format: format, quality: quality, admit: admit,
      makeImage: {
        guard let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else { throw MediaError.Fetch(detail: "the bytes are not an image") }
        return image
      },
      sink: { try ImagePreparation.encoded($0, format: format, quality: quality) }
    )
  }

  static func run<Result>(
    _ image: CGImage, exifOrientation: UInt32, format: ImageFormat, quality: Double, admit: ImagePreparation.Admit, sink: (CGImage) throws -> Result
  ) async throws -> Result {
    let alpha: Bool
    switch image.alphaInfo {
    case .none, .noneSkipFirst, .noneSkipLast: alpha = false
    default: alpha = true
    }
    return try await run(
      width: UInt32(clamping: image.width), height: UInt32(clamping: image.height), exifOrientation: exifOrientation,
      hasAlpha: alpha, format: format, quality: quality, admit: admit, makeImage: { image }, sink: sink
    )
  }

  /// The memory is asked of the budget first, `makeImage` runs after the admission, `sink` takes the finished image inside it.
  static func run<Result>(
    width: UInt32, height: UInt32, exifOrientation: UInt32, hasAlpha: Bool, format: ImageFormat, quality: Double,
    admit: ImagePreparation.Admit, makeImage: () throws -> CGImage, sink: (CGImage) throws -> Result
  ) async throws -> Result {
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
    return try sink(result)
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

/// The shared writers of an image in memory: the encoders and the turn used by `encodeImage`.
enum ImagePreparation {
  typealias Admit = @Sendable (UInt64) async throws -> @Sendable () -> Void

  static func encoded(_ image: CGImage, format: ImageFormat, quality: Double) throws -> Data {
    let data = NSMutableData()
    guard let destination = CGImageDestinationCreateWithData(data, imageType(format).identifier as CFString, 1, nil) else {
      throw MediaError.Unexpected(detail: "the image destination cannot be created")
    }
    try finalize(destination, image, format: format, quality: quality)
    return data as Data
  }

  /// The encoder writes into `output` as it goes, the encoded file is never held whole in memory.
  static func encode(_ image: CGImage, format: ImageFormat, quality: Double, to output: URL) throws {
    guard let destination = CGImageDestinationCreateWithURL(output as CFURL, imageType(format).identifier as CFString, 1, nil) else {
      throw MediaError.Unexpected(detail: "the image destination cannot be created")
    }
    try finalize(destination, image, format: format, quality: quality)
  }

  private static func imageType(_ format: ImageFormat) -> UTType { format == .png ? .png : .jpeg }

  private static func finalize(_ destination: CGImageDestination, _ image: CGImage, format: ImageFormat, quality: Double) throws {
    let options: CFDictionary? = format == .png ? nil : [kCGImageDestinationLossyCompressionQuality: jpegQuality(quality: quality)] as CFDictionary
    CGImageDestinationAddImage(destination, image, options)
    guard CGImageDestinationFinalize(destination) else { throw MediaError.Unexpected(detail: "the image cannot be encoded") }
  }

  static func draw(_ image: CGImage, turn: PhotoTurn) throws -> CGImage {
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
