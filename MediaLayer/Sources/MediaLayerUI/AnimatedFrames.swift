import CoreGraphics
import Foundation
import ImageIO
import SwiftUI

final class FrameStream: @unchecked Sendable {
  static let window = 3

  private let source: CGImageSource
  private let options: CFDictionary
  private let count: Int
  private let lock = NSLock()
  private var cache: [Int: CGImage] = [:]

  init(source: CGImageSource, options: CFDictionary, count: Int) {
    self.source = source
    self.options = options
    self.count = count
  }

  var cachedCount: Int { lock.withLock { cache.count } }

  func image(at index: Int) -> CGImage? {
    lock.withLock {
      if let hit = cache[index] { return hit }
      guard let image = CGImageSourceCreateThumbnailAtIndex(source, index, options) else { return nil }

      let keep = Set((0 ..< Self.window).map { (index + $0) % count })
      cache = cache.filter { keep.contains($0.key) }
      cache[index] = image
      return image
    }
  }
}

struct AnimatedFrames {
  enum Storage {
    case decoded([CGImage])
    case streamed(FrameStream)
  }

  static let minimumDelay: TimeInterval = 0.02
  static let defaultDelay: TimeInterval = 0.1
  static let maxPixelLimit = 1024
  static let byteBudget = 32 << 20

  let token = UUID()
  let storage: Storage
  let frameCount: Int
  let firstImage: CGImage
  let duration: TimeInterval

  func loopDuration(rate: Double) -> TimeInterval { rate > 0 ? duration / rate : duration }

  func image(at index: Int) -> CGImage? {
    switch storage {
    case let .decoded(images): images.indices.contains(index) ? images[index] : nil
    case let .streamed(stream): stream.image(at: index)
    }
  }

  static func load(_ data: Data, maxPixel: Int) async -> AnimatedFrames? {
    decode(data, maxPixel: maxPixel, isCancelled: { Task.isCancelled })
  }

  static func decode(_ data: Data, maxPixel: Int, byteBudget: Int = byteBudget, isCancelled: () -> Bool = { false }) -> AnimatedFrames? {
    guard let source = CGImageSourceCreateWithData(data as CFData, nil) else { return nil }

    let count = CGImageSourceGetCount(source)
    let options: [CFString: Any] = [
      kCGImageSourceCreateThumbnailFromImageAlways: true,
      kCGImageSourceCreateThumbnailWithTransform: true,
      kCGImageSourceShouldCacheImmediately: true,
      kCGImageSourceThumbnailMaxPixelSize: min(max(maxPixel, 1), maxPixelLimit)
    ]
    guard count > 1, let first = CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) else { return nil }

    var duration: TimeInterval = 0
    for index in 0 ..< count {
      if isCancelled() { return nil }
      duration += delay(source: source, index: index)
    }
    if first.bytesPerRow * first.height * count > byteBudget {
      let stream = FrameStream(source: source, options: options as CFDictionary, count: count)
      return AnimatedFrames(storage: .streamed(stream), frameCount: count, firstImage: first, duration: duration)
    }

    var images = [first]
    for index in 1 ..< count {
      if isCancelled() { return nil }
      guard let image = CGImageSourceCreateThumbnailAtIndex(source, index, options as CFDictionary) else { continue }

      images.append(image)
    }
    guard images.count > 1 else { return nil }

    return AnimatedFrames(storage: .decoded(images), frameCount: images.count, firstImage: first, duration: duration)
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
    var stream: Task<Void, Never>?
    var streamInterval: TimeInterval = 0
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
    let playback = context.coordinator
    if playback.token != frames.token {
      playback.token = frames.token
      playback.stream?.cancel()
      playback.stream = nil
      view.stopAnimating()
      switch frames.storage {
      case let .decoded(images):
        view.image = nil
        view.animationImages = images.map { UIImage(cgImage: $0) }
      case .streamed:
        view.animationImages = nil
        view.image = UIImage(cgImage: frames.firstImage)
      }
    }
    switch frames.storage {
    case .decoded:
      view.animationDuration = frames.loopDuration(rate: playbackRate)
      if isPlaying {
        if !view.isAnimating { view.startAnimating() }
      } else {
        view.stopAnimating()
      }
    case let .streamed(stream):
      let interval = frames.loopDuration(rate: playbackRate) / Double(frames.frameCount)
      if !isPlaying {
        playback.stream?.cancel()
        playback.stream = nil
      } else if playback.stream == nil || playback.streamInterval != interval {
        playback.stream?.cancel()
        playback.streamInterval = interval
        playback.stream = Self.play(stream, count: frames.frameCount, interval: interval, on: view)
      }
    }
  }

  static func dismantleUIView(_ view: UIImageView, coordinator: Playback) {
    coordinator.stream?.cancel()
    coordinator.stream = nil
  }

  private static func play(_ stream: FrameStream, count: Int, interval: TimeInterval, on view: UIImageView) -> Task<Void, Never> {
    Task { @MainActor [weak view] in
      var index = 0
      while !Task.isCancelled, let view {
        let next = (index + 1) % count
        let prefetch = Task.detached { stream.image(at: next) }
        if let image = await Task.detached(operation: { stream.image(at: index) }).value { view.image = UIImage(cgImage: image) }
        try? await Task.sleep(for: .seconds(interval))
        _ = await prefetch.value
        index = next
      }
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
    Image(decorative: frames.firstImage, scale: 1).resizable().aspectRatio(contentMode: contentMode)
  }
}
#endif
