import AVFoundation
import CoreGraphics
import Foundation
import MediaLayer

enum ToneFrames {
  static let longSide = 64

  static func rgba(from image: CGImage) -> (width: Int, height: Int, rgba: Data)? {
    let long = max(image.width, image.height)
    guard image.width > 0, image.height > 0 else { return nil }

    let scale = min(1, Double(longSide) / Double(long))
    let width = max(1, Int((Double(image.width) * scale).rounded()))
    let height = max(1, Int((Double(image.height) * scale).rounded()))
    var bytes = Data(count: width * height * 4)
    let drawn = bytes.withUnsafeMutableBytes { buffer -> Bool in
      guard let space = CGColorSpace(name: CGColorSpace.sRGB),
            let context = CGContext(
              data: buffer.baseAddress, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
              space: space, bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
            ) else { return false }
      context.interpolationQuality = .medium
      context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
      return true
    }
    return drawn ? (width, height, bytes) : nil
  }

  static func submit(_ image: CGImage?, source: MediaItem, places: MediaPlaces) {
    guard let image else { return }

    Task.detached(priority: .utility) {
      guard let pixels = rgba(from: image) else { return }

      places.submitTonePixels(source: source, width: UInt32(pixels.width), height: UInt32(pixels.height), rgba: pixels.rgba)
    }
  }

  static func submitFirstFrame(of source: MediaItem, slots: LoadedSlots?, places: MediaPlaces) {
    guard source.cover == nil, let asset = slots?.slot(for: source.url)?.loadedItem?.asset else { return }

    submitFirstFrame(of: asset, source: source, places: places)
  }

  static func submitFirstFrame(of asset: AVAsset, source: MediaItem, places: MediaPlaces) {
    Task.detached(priority: .utility) {
      let generator = AVAssetImageGenerator(asset: asset)
      generator.appliesPreferredTrackTransform = true
      generator.maximumSize = CGSize(width: longSide, height: longSide)
      guard let frame = try? await generator.image(at: .zero).image else { return }

      submit(frame, source: source, places: places)
    }
  }
}

extension PlatformImage {
  var toneImage: CGImage? {
    #if canImport(UIKit)
    cgImage
    #else
    cgImage(forProposedRect: nil, context: nil, hints: nil)
    #endif
  }
}
