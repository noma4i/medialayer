import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

extension MediaLayer {
  /// A photo made upright by its EXIF orientation and saved as JPEG. No metadata is copied, so the editor tag and every EXIF field are gone.
  /// `maxSize` fits the result into the rectangle and keeps the proportions, a smaller result is enlarged only when `enlarge` is true, `outputSize` gives the exact size and wins over `maxSize`,
  /// `rotation` turns the upright photo clockwise by a multiple of 90 degrees (negative turns the other way) before `crop`,
  /// `crop` is a rectangle in fractions 0 to 1 of the turned photo (origin top left), `mirror` flips the cropped result horizontally.
  /// A rotation that is not a multiple of 90 is refused with `MediaError.Fetch`.
  /// An edit that holds no pixel, such as an empty crop or a size of zero, is refused with `MediaError.Fetch`. The core plans every size.
  public func prepareImage(
    _ source: some MediaSource,
    quality: Double = 0.9,
    maxSize: CGSize? = nil,
    enlarge: Bool = false,
    crop: CGRect? = nil,
    outputSize: CGSize? = nil,
    mirror: Bool = false,
    rotation: Int = 0
  ) async throws -> PreparedMedia {
    let file = try preparableFile(normalizeItem(item: source.item).url)
    let edit = PhotoEdit(
      maxSize: maxSize.map(pixelSize),
      enlarge: enlarge,
      crop: crop.map { CropFractions(x: Double($0.minX), y: Double($0.minY), width: Double($0.width), height: Double($0.height)) },
      outputSize: outputSize.map(pixelSize),
      mirror: mirror,
      rotationDegrees: UInt32(clamping: ((rotation % 360) + 360) % 360)
    )
    let work = Task.detached {
      try await ImagePreparation.run(file, quality: quality, edit: edit, counters: self.imageStats()) { bytes in
        let hold = try await self.reserveDecode(bytes: bytes)
        return { hold.release() }
      }
    }
    return try await withTaskCancellationHandler { try await work.value } onCancel: { work.cancel() }
  }
}

private func pixelSize(_ size: CGSize) -> PixelSize {
  let side = { (value: CGFloat) -> UInt32 in value.isFinite ? UInt32(clamping: Int(max(0, min(value.rounded(), 1_000_000)))) : 0 }
  return PixelSize(width: side(size.width), height: side(size.height))
}

enum ImagePreparation {
  typealias Admit = @Sendable (UInt64) async throws -> @Sendable () -> Void

  static let plain = PhotoEdit(maxSize: nil, enlarge: false, crop: nil, outputSize: nil, mirror: false, rotationDegrees: 0)

  static func run(_ file: URL, quality: Double, edit: PhotoEdit = plain, directory: URL = FileManager.default.temporaryDirectory, counters: ImageCounters? = nil, admit: Admit) async throws -> PreparedMedia {
    guard let source = CGImageSourceCreateWithURL(file as CFURL, nil),
          let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
          let width = properties[kCGImagePropertyPixelWidth] as? Int,
          let height = properties[kCGImagePropertyPixelHeight] as? Int
    else { throw MediaError.Fetch(detail: "\(file.path) is not an image") }
    let tag = (properties[kCGImagePropertyOrientation] as? UInt32) ?? 1
    guard let plan = photoPlan(width: UInt32(clamping: width), height: UInt32(clamping: height), exifOrientation: tag, edit: edit) else {
      throw MediaError.Fetch(detail: "\(file.path) is above the pixel limit or the edit holds no pixel")
    }
    let release = try await admit(plan.cost)
    defer { release() }
    var result = try upright(source, plan: plan, path: file.path)
    counters?.record(work: .decode(width: UInt32(result.width), height: UInt32(result.height), bytes: UInt64(result.width * result.height * 4)))
    if edit.rotationDegrees % 360 != 0 { result = try draw(result, turn: PhotoTurn(rotationDegrees: edit.rotationDegrees % 360, flipHorizontal: false)) }
    try Task.checkCancellation()
    if plan.needsDraw { result = try finish(result, plan: plan) }
    try Task.checkCancellation()
    let output = preparedOutput(extension: "jpg", in: directory)
    do {
      try encoded(result, format: .jpeg, quality: quality).write(to: output)
    } catch {
      try? FileManager.default.removeItem(at: output)
      throw error as? MediaError ?? MediaError.Unexpected(detail: "the JPEG cannot be written")
    }
    return PreparedMedia(url: output, isNew: true)
  }

  /// The photo decoded upright at the thumbnail side of the plan, so a large crop never holds more pixels than its output asks for.
  static func upright(_ source: CGImageSource, plan: PhotoPlan, path: String) throws -> CGImage {
    let options: [CFString: Any] = [
      kCGImageSourceCreateThumbnailFromImageAlways: true,
      kCGImageSourceCreateThumbnailWithTransform: true,
      kCGImageSourceShouldCacheImmediately: true,
      kCGImageSourceThumbnailMaxPixelSize: plan.thumbnailSide
    ]
    guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { throw MediaError.Fetch(detail: "\(path) is not an image") }
    return image
  }

  /// The crop of the plan, moved to the decoded size of `image`, drawn into the output size and flipped when the plan asks for the mirror.
  static func finish(_ image: CGImage, plan: PhotoPlan) throws -> CGImage {
    let across = Double(image.width) / Double(plan.upright.width)
    let down = Double(image.height) / Double(plan.upright.height)
    let left = min(max(Int((Double(plan.crop.x) * across).rounded()), 0), image.width - 1)
    let top = min(max(Int((Double(plan.crop.y) * down).rounded()), 0), image.height - 1)
    let right = min(max(Int((Double(plan.crop.x + plan.crop.width) * across).rounded()), left + 1), image.width)
    let bottom = min(max(Int((Double(plan.crop.y + plan.crop.height) * down).rounded()), top + 1), image.height)
    let area = CGRect(x: left, y: top, width: right - left, height: bottom - top)
    guard let cropped = image.cropping(to: area) else { throw MediaError.Unexpected(detail: "the photo cannot be cropped") }
    guard let context = CGContext(
      data: nil, width: Int(plan.output.width), height: Int(plan.output.height), bitsPerComponent: 8, bytesPerRow: 0,
      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
    ) else { throw MediaError.Unexpected(detail: "the photo canvas cannot be created") }
    context.interpolationQuality = .high
    if plan.mirror {
      context.translateBy(x: CGFloat(plan.output.width), y: 0)
      context.scaleBy(x: -1, y: 1)
    }
    context.draw(cropped, in: CGRect(x: 0, y: 0, width: Int(plan.output.width), height: Int(plan.output.height)))
    guard let result = context.makeImage() else { throw MediaError.Unexpected(detail: "the photo cannot be drawn") }
    return result
  }

  static func encoded(_ image: CGImage, format: ImageFormat, quality: Double) throws -> Data {
    let data = NSMutableData()
    let type = format == .png ? UTType.png : UTType.jpeg
    guard let destination = CGImageDestinationCreateWithData(data, type.identifier as CFString, 1, nil) else {
      throw MediaError.Unexpected(detail: "the image destination cannot be created")
    }
    let options: CFDictionary? = format == .png ? nil : [kCGImageDestinationLossyCompressionQuality: jpegQuality(quality: quality)] as CFDictionary
    CGImageDestinationAddImage(destination, image, options)
    guard CGImageDestinationFinalize(destination) else { throw MediaError.Unexpected(detail: "the image cannot be encoded") }
    return data as Data
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
