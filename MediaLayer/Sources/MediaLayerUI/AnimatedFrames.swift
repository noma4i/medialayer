import CoreGraphics
import Foundation
import ImageIO
import SwiftUI

struct AnimatedFrames {
  static let minimumDelay: TimeInterval = 0.02
  static let defaultDelay: TimeInterval = 0.1
  static let maxPixelLimit = 1024

  let token = UUID()
  let images: [CGImage]
  let duration: TimeInterval

  func loopDuration(rate: Double) -> TimeInterval { rate > 0 ? duration / rate : duration }

  static func load(_ data: Data, maxPixel: Int) async -> AnimatedFrames? {
    decode(data, maxPixel: maxPixel)
  }

  static func decode(_ data: Data, maxPixel: Int) -> AnimatedFrames? {
    guard let source = CGImageSourceCreateWithData(data as CFData, nil), CGImageSourceGetCount(source) > 1 else { return nil }

    let options: [CFString: Any] = [
      kCGImageSourceCreateThumbnailFromImageAlways: true,
      kCGImageSourceCreateThumbnailWithTransform: true,
      kCGImageSourceShouldCacheImmediately: true,
      kCGImageSourceThumbnailMaxPixelSize: min(max(maxPixel, 1), maxPixelLimit)
    ]
    var images: [CGImage] = []
    var duration: TimeInterval = 0
    for index in 0 ..< CGImageSourceGetCount(source) {
      guard let image = CGImageSourceCreateThumbnailAtIndex(source, index, options as CFDictionary) else { continue }

      images.append(image)
      duration += delay(source: source, index: index)
    }
    return images.count > 1 ? AnimatedFrames(images: images, duration: duration) : nil
  }

  private static func delay(source: CGImageSource, index: Int) -> TimeInterval {
    let properties = CGImageSourceCopyPropertiesAtIndex(source, index, nil) as? [CFString: Any]
    let gif = properties?[kCGImagePropertyGIFDictionary] as? [CFString: Any]
    let value = (gif?[kCGImagePropertyGIFUnclampedDelayTime] as? Double) ?? (gif?[kCGImagePropertyGIFDelayTime] as? Double) ?? 0
    return value < minimumDelay ? defaultDelay : value
  }
}

#if canImport(UIKit)
import UIKit

struct AnimatedFramesView: UIViewRepresentable {
  final class Playback {
    var token: UUID?
  }

  let frames: AnimatedFrames
  let contentMode: ContentMode
  let isPlaying: Bool
  let playbackRate: Double

  func makeCoordinator() -> Playback { Playback() }

  func makeUIView(context: Context) -> UIImageView {
    let view = UIImageView()
    view.isOpaque = false
    view.backgroundColor = .clear
    view.clipsToBounds = true
    return view
  }

  func updateUIView(_ view: UIImageView, context: Context) {
    view.contentMode = contentMode == .fill ? .scaleAspectFill : .scaleAspectFit
    if context.coordinator.token != frames.token {
      context.coordinator.token = frames.token
      view.stopAnimating()
      view.animationImages = frames.images.map { UIImage(cgImage: $0) }
    }
    view.animationDuration = frames.loopDuration(rate: playbackRate)
    if isPlaying {
      if !view.isAnimating { view.startAnimating() }
    } else {
      view.stopAnimating()
    }
  }

  func sizeThatFits(_ proposal: ProposedViewSize, uiView: UIImageView, context: Context) -> CGSize? {
    proposal.replacingUnspecifiedDimensions()
  }
}
#else
struct AnimatedFramesView: View {
  let frames: AnimatedFrames
  let contentMode: ContentMode
  let isPlaying: Bool
  let playbackRate: Double

  var body: some View {
    Image(decorative: frames.images[0], scale: 1).resizable().aspectRatio(contentMode: contentMode)
  }
}
#endif
