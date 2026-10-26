#if canImport(UIKit)
import MediaLayer
import UIKit

extension MediaLayer {
  /// An image in memory saved as a JPEG or PNG file, turned upright by its `imageOrientation`. See `encodeImage(_:exifOrientation:format:quality:)`.
  public func encodeImage(_ image: UIImage, format: ImageFormat = .jpeg, quality: Double = 0.9) async throws -> PreparedMedia {
    guard let pixels = image.cgImage else { throw MediaError.Fetch(detail: "the image holds no pixels") }
    return try await encodeImage(pixels, exifOrientation: image.imageOrientation.exifTag, format: format, quality: quality)
  }
}

extension UIImage.Orientation {
  var exifTag: UInt32 {
    switch self {
    case .up: 1
    case .upMirrored: 2
    case .down: 3
    case .downMirrored: 4
    case .leftMirrored: 5
    case .right: 6
    case .rightMirrored: 7
    case .left: 8
    @unknown default: 1
    }
  }
}
#endif
