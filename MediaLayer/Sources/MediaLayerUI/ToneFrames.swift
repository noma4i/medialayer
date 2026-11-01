import AVFoundation
import CoreGraphics
import Foundation
import ImageIO
import MediaLayer

final class ToneFrames: TonePreparation, @unchecked Sendable {
  static let longSide = 64
  private weak var places: MediaPlaces?

  private let thumbnail: @Sendable (Data) -> CGImage?

  init(places: MediaPlaces, thumbnail: @escaping @Sendable (Data) -> CGImage? = { ToneFrames.thumbnail($0) }) {
    self.places = places
    self.thumbnail = thumbnail
  }

  func prepare(source: MediaItem) async -> TonePixels? {
    guard let places, await places.tonePixelsNeeded(source: source), !Task.isCancelled else { return nil }
    let image: CGImage?
    if source.kind == .image || source.cover != nil {
      image = await self.image(source: source, places: places)
    } else {
      image = await firstFrame(source: source, places: places)
    }
    guard !Task.isCancelled, let image, let pixels = Self.rgba(from: image) else { return nil }
    return TonePixels(width: UInt32(pixels.width), height: UInt32(pixels.height), rgba: pixels.rgba)
  }

  private func image(source: MediaItem, places: MediaPlaces) async -> CGImage? {
    guard let url = URL(string: source.cover ?? source.url) else { return nil }
    let stats = places.imageStats()
    stats.record(work: .request(placementPixels: 0))
    defer { stats.record(work: Task.isCancelled ? .cancel : .finish(memoryHit: false)) }
    let result = try? await places.fetchImage(url: url.absoluteString, headers: source.headers)
    if result?.source == .cache { stats.record(work: .diskHit) }
    guard !Task.isCancelled, let data = result?.bytes else { return nil }
    let image = thumbnail(data)
    if let image { stats.record(work: .decode(width: UInt32(image.width), height: UInt32(image.height), bytes: UInt64(image.bytesPerRow * image.height))) }
    return image
  }

  private func firstFrame(source: MediaItem, places: MediaPlaces) async -> CGImage? {
    guard source.kind == .video, let route = try? await places.playUrl(source: source), let url = URL(string: route.url) else { return nil }
    let asset = AVURLAsset(url: url, options: ["AVURLAssetHTTPHeaderFieldsKey": route.headers])
    let generator = AVAssetImageGenerator(asset: asset)
    generator.appliesPreferredTrackTransform = true
    generator.maximumSize = CGSize(width: Self.longSide, height: Self.longSide)
    return await withTaskCancellationHandler {
      try? await generator.image(at: .zero).image
    } onCancel: { generator.cancelAllCGImageGeneration() }
  }

  static func thumbnail(_ data: Data) -> CGImage? {
    guard let encoded = CGImageSourceCreateWithData(data as CFData, [kCGImageSourceShouldCache: false] as CFDictionary) else { return nil }
    return CGImageSourceCreateThumbnailAtIndex(encoded, 0, [
      kCGImageSourceCreateThumbnailFromImageAlways: true,
      kCGImageSourceCreateThumbnailWithTransform: true,
      kCGImageSourceThumbnailMaxPixelSize: longSide,
      kCGImageSourceShouldCacheImmediately: true
    ] as CFDictionary)
  }

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

}
