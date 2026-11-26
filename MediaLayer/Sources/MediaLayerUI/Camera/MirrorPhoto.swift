import CoreGraphics
import Foundation
import ImageIO
import MediaLayer
import UniformTypeIdentifiers

/// The horizontal mirror of an upright JPEG file, written in place after the EXIF turn is applied.
enum MirrorPhoto {
  static func run(_ url: URL, quality: Double) throws {
    let failed = MediaError.Camera(reason: .photoFailed, detail: cameraRefusalText(reason: .photoFailed), recordedMs: nil)
    guard let source = CGImageSourceCreateWithURL(url as CFURL, nil), let image = CGImageSourceCreateImageAtIndex(source, 0, nil) else { throw failed }
    let width = image.width
    let height = image.height
    guard let context = CGContext(data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpace(name: CGColorSpace.sRGB) ?? CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue) else { throw failed }
    context.translateBy(x: CGFloat(width), y: 0)
    context.scaleBy(x: -1, y: 1)
    context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
    guard let flipped = context.makeImage(), let destination = CGImageDestinationCreateWithURL(url as CFURL, UTType.jpeg.identifier as CFString, 1, nil) else { throw failed }
    CGImageDestinationAddImage(destination, flipped, [kCGImageDestinationLossyCompressionQuality: quality] as CFDictionary)
    guard CGImageDestinationFinalize(destination) else { throw failed }
  }
}
