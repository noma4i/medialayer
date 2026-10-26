import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers

extension MediaLayer {
  /// A photo made upright by its EXIF orientation and saved as JPEG. No metadata is copied, so the editor tag and every EXIF field are gone.
  /// `maxSize` fits the result into the rectangle (never enlarged), `outputSize` gives the exact size and wins over `maxSize`,
  /// `crop` is a rectangle in fractions 0 to 1 of the upright photo (origin top left), `mirror` flips the cropped result horizontally.
  /// An edit that holds no pixel, such as an empty crop or a size of zero, is refused with `MediaError.Fetch`. The core plans every size.
  public func prepareImage(
    _ source: some MediaSource,
    quality: Double = 0.9,
    maxSize: CGSize? = nil,
    crop: CGRect? = nil,
    outputSize: CGSize? = nil,
    mirror: Bool = false
  ) async throws -> PreparedMedia {
    let file = try preparableFile(normalizeItem(item: source.item).url)
    let edit = PhotoEdit(
      maxSize: maxSize.map(pixelSize),
      crop: crop.map { CropFractions(x: Double($0.minX), y: Double($0.minY), width: Double($0.width), height: Double($0.height)) },
      outputSize: outputSize.map(pixelSize),
      mirror: mirror
    )
    let work = Task.detached {
      try await ImagePreparation.run(file, quality: quality, edit: edit) { bytes in
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

  static let plain = PhotoEdit(maxSize: nil, crop: nil, outputSize: nil, mirror: false)

  static func run(_ file: URL, quality: Double, edit: PhotoEdit = plain, directory: URL = FileManager.default.temporaryDirectory, admit: Admit) async throws -> PreparedMedia {
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
    guard let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else { throw MediaError.Fetch(detail: "\(file.path) is not an image") }
    try Task.checkCancellation()
    var result = try draw(image, turn: plan.turn)
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

  /// The crop of the plan drawn into the output size, flipped when the plan asks for the mirror.
  static func finish(_ image: CGImage, plan: PhotoPlan) throws -> CGImage {
    let area = CGRect(x: Int(plan.crop.x), y: Int(plan.crop.y), width: Int(plan.crop.width), height: Int(plan.crop.height))
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
