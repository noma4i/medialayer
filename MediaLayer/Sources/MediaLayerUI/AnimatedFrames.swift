import CoreGraphics
import Foundation
import ImageIO
import SwiftUI

final class FrameStream: @unchecked Sendable {
  static let window = 3

  private let source: CGImageSource
  private let options: CFDictionary
  private let lock = NSLock()
  private var cachedIndex: Int?
  private var cachedImage: CGImage?
  private var preparationCount = 0
  private var decodeCount = 0

  init(source: CGImageSource, options: CFDictionary) {
    self.source = source
    self.options = options
  }

  var cachedCount: Int { lock.withLock { cachedImage == nil ? 0 : 1 } }
  var preparations: Int { lock.withLock { preparationCount } }
  var decodes: Int { lock.withLock { decodeCount } }

  func preparedTask() { lock.withLock { preparationCount += 1 } }

  func image(at index: Int) -> CGImage? {
    lock.withLock {
      if cachedIndex == index { return cachedImage }
      cachedImage = nil
      cachedIndex = nil
      decodeCount += 1
      guard let image = CGImageSourceCreateThumbnailAtIndex(source, index, options) else { return nil }

      cachedIndex = index
      cachedImage = image
      return image
    }
  }
}

struct AnimatedFrames {
  static let minimumDelay: TimeInterval = 0.02
  static let defaultDelay: TimeInterval = 0.1
  static let maxPixelLimit = 1024

  let token = UUID()
  let stream: FrameStream
  let frameCount: Int
  let firstImage: CGImage
  let duration: TimeInterval
  let frameEnds: [TimeInterval]

  let decodedBytes: Int

  func loopDuration(rate: Double) -> TimeInterval { rate > 0 ? duration / rate : duration }

  func frameTick(elapsed: TimeInterval, rate: Double) -> Int {
    let position = max(0, elapsed) * (rate > 0 ? rate : 1)
    let round = Int(position / duration)
    let offset = position - Double(round) * duration
    var low = 0
    var high = frameCount
    while low < high {
      let middle = (low + high) / 2
      if offset < frameEnds[middle] { high = middle } else { low = middle + 1 }
    }
    return round * frameCount + min(low, frameCount - 1)
  }

  func frameEnd(tick: Int, rate: Double) -> TimeInterval {
    (Double(tick / frameCount) * duration + frameEnds[tick % frameCount]) / (rate > 0 ? rate : 1)
  }

  func image(at index: Int) -> CGImage? {
    index == 0 ? firstImage : stream.image(at: index)
  }

  static func load(_ data: Data, maxPixel: Int) async -> AnimatedFrames? {
    decode(data, maxPixel: maxPixel, isCancelled: { Task.isCancelled })
  }

  static func decode(_ data: Data, maxPixel: Int, isCancelled: () -> Bool = { false }) -> AnimatedFrames? {
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
    var frameEnds: [TimeInterval] = []
    for index in 0 ..< count {
      if isCancelled() { return nil }
      duration += delay(source: source, index: index)
      frameEnds.append(duration)
    }
    let stream = FrameStream(source: source, options: options as CFDictionary)
    return AnimatedFrames(stream: stream, frameCount: count, firstImage: first, duration: duration, frameEnds: frameEnds, decodedBytes: first.bytesPerRow * first.height * FrameStream.window)
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
    private var task: Task<Void, Never>?
    private var runningInterval: TimeInterval = 0

    deinit { task?.cancel() }

    func sync(_ view: UIImageView) {
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
      view.image = UIImage(cgImage: frames.firstImage)
    }
    playback.plan = isPlaying ? StreamPlan(stream: frames.stream, frames: frames, rate: playbackRate) : nil
    playback.sync(view)
  }

  static func dismantleUIView(_ view: UIImageView, coordinator: Playback) {
    coordinator.plan = nil
    coordinator.stop()
  }

  private static func play(_ plan: StreamPlan, on view: UIImageView) -> Task<Void, Never> {
    plan.stream.preparedTask()
    return Task.detached { [weak view] in
      let clock = ContinuousClock()
      let start = clock.now
      var shown = -1
      while !Task.isCancelled {
        let parts = (clock.now - start).components
        let tick = plan.frames.frameTick(elapsed: Double(parts.seconds) + Double(parts.attoseconds) * 1e-18, rate: plan.rate)
        let index = tick % plan.frames.frameCount
        if index != shown {
          let image = plan.frames.image(at: index)
          guard !Task.isCancelled else { return }

          await MainActor.run { [weak view] in
            if !Task.isCancelled, let image { view?.image = UIImage(cgImage: image) }
          }
          shown = index
        }
        let upcoming = (tick + 1) % plan.frames.frameCount
        guard !Task.isCancelled else { return }
        _ = plan.frames.image(at: upcoming)
        try? await clock.sleep(until: start + .seconds(plan.frames.frameEnd(tick: tick, rate: plan.rate)))
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
