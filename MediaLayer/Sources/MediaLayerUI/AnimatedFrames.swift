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

  var decodedBytes: Int {
    guard case let .decoded(images) = storage else { return 0 }

    return images.reduce(0) { $0 + $1.bytesPerRow * $1.height }
  }

  func loopDuration(rate: Double) -> TimeInterval { rate > 0 ? duration / rate : duration }

  func frameTick(elapsed: TimeInterval, rate: Double) -> Int {
    Int(elapsed / (loopDuration(rate: rate) / Double(frameCount)))
  }

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

final class FrameImageView: UIImageView {
  var windowChanged: (() -> Void)?

  override func didMoveToWindow() {
    super.didMoveToWindow()
    windowChanged?()
  }
}

struct AnimatedFramesView: UIViewRepresentable {
  struct StreamPlan {
    let stream: FrameStream
    let frames: AnimatedFrames
    let rate: Double

    var interval: TimeInterval { frames.loopDuration(rate: rate) / Double(frames.frameCount) }
  }

  final class Playback {
    var token: UUID?
    var plan: StreamPlan?
    var animatesImages = false
    private var task: Task<Void, Never>?
    private var runningInterval: TimeInterval = 0

    func sync(_ view: UIImageView) {
      if animatesImages, view.window != nil {
        if !view.isAnimating { view.startAnimating() }
      } else if view.isAnimating {
        view.stopAnimating()
      }
      guard let plan, view.window != nil else { return stop() }
      guard task == nil || runningInterval != plan.interval else { return }

      stop()
      runningInterval = plan.interval
      task = AnimatedFramesView.play(plan, on: view)
    }

    func stop() {
      task?.cancel()
      task = nil
    }
  }

  let frames: AnimatedFrames
  let contentMode: ContentMode
  let isPlaying: Bool
  let playbackRate: Double

  func makeCoordinator() -> Playback { Playback() }

  func makeUIView(context: Context) -> UIImageView {
    let view = FrameImageView()
    view.isOpaque = false
    view.backgroundColor = .clear
    view.clipsToBounds = true
    return view
  }

  func updateUIView(_ view: UIImageView, context: Context) {
    update(view, playback: context.coordinator)
  }

  func update(_ view: UIImageView, playback: Playback) {
    view.contentMode = contentMode == .fill ? .scaleAspectFill : .scaleAspectFit
    (view as? FrameImageView)?.windowChanged = { [weak playback, weak view] in
      if let playback, let view { playback.sync(view) }
    }
    if playback.token != frames.token {
      playback.token = frames.token
      playback.stop()
      view.stopAnimating()
      switch frames.storage {
      case let .decoded(images):
        view.image = UIImage(cgImage: frames.firstImage)
        view.animationImages = images.map { UIImage(cgImage: $0) }
      case .streamed:
        view.animationImages = nil
        view.image = UIImage(cgImage: frames.firstImage)
      }
    }
    switch frames.storage {
    case .decoded:
      playback.plan = nil
      playback.animatesImages = isPlaying
      view.animationDuration = frames.loopDuration(rate: playbackRate)
    case let .streamed(stream):
      playback.animatesImages = false
      playback.plan = isPlaying ? StreamPlan(stream: stream, frames: frames, rate: playbackRate) : nil
    }
    playback.sync(view)
  }

  static func dismantleUIView(_ view: UIImageView, coordinator: Playback) {
    coordinator.plan = nil
    coordinator.animatesImages = false
    coordinator.stop()
  }

  private static func play(_ plan: StreamPlan, on view: UIImageView) -> Task<Void, Never> {
    Task { @MainActor [weak view] in
      let clock = ContinuousClock()
      let start = clock.now
      var shown = -1
      while !Task.isCancelled, let view {
        let parts = (clock.now - start).components
        let tick = plan.frames.frameTick(elapsed: Double(parts.seconds) + Double(parts.attoseconds) * 1e-18, rate: plan.rate)
        let index = tick % plan.frames.frameCount
        if index != shown {
          let image = await Task.detached(operation: { plan.stream.image(at: index) }).value
          guard !Task.isCancelled else { return }

          if let image { view.image = UIImage(cgImage: image) }
          shown = index
        }
        let upcoming = (tick + 1) % plan.frames.frameCount
        Task.detached { _ = plan.stream.image(at: upcoming) }
        try? await clock.sleep(until: start + .seconds(Double(tick + 1) * plan.interval))
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
