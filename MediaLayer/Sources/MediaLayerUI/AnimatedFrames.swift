import CoreGraphics
import Foundation
import ImageIO
import MediaLayer
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
    let result = lock.withLock { () -> (CGImage?, Bool) in
      if cachedIndex == index { return (cachedImage, false) }
      cachedImage = nil
      cachedIndex = nil
      decodeCount += 1
      guard let image = CGImageSourceCreateThumbnailAtIndex(source, index, options) else { return (nil, true) }

      cachedIndex = index
      cachedImage = image
      return (image, true)
    }
    if result.1 { TestChange.fire() }
    return result.0
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

  let retainedBytes: Int

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

  static func decodeBytes(sourceBytes: Int, maxPixel: Int) -> UInt64 {
    let side = UInt32(min(max(maxPixel, 1), maxPixelLimit))
    return UInt64(sourceBytes) + 2 * (decodeCost(width: side, height: side) ?? 0)
  }

  static func sourceFrameCount(_ data: Data) -> Int {
    CGImageSourceCreateWithData(data as CFData, nil).map { CGImageSourceGetCount($0) } ?? 0
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
    return AnimatedFrames(stream: stream, frameCount: count, firstImage: first, duration: duration, frameEnds: frameEnds, retainedBytes: first.bytesPerRow * first.height * FrameStream.window + data.count)
  }

  private static func delay(source: CGImageSource, index: Int) -> TimeInterval {
    let properties = CGImageSourceCopyPropertiesAtIndex(source, index, nil) as? [CFString: Any]
    let gif = properties?[kCGImagePropertyGIFDictionary] as? [CFString: Any]
    let webp = properties?[kCGImagePropertyWebPDictionary] as? [CFString: Any]
    let value = (gif?[kCGImagePropertyGIFUnclampedDelayTime] as? Double) ?? (gif?[kCGImagePropertyGIFDelayTime] as? Double)
      ?? (webp?[kCGImagePropertyWebPUnclampedDelayTime] as? Double) ?? (webp?[kCGImagePropertyWebPDelayTime] as? Double) ?? 0
    return value < minimumDelay ? defaultDelay : value
  }
}

#if canImport(UIKit)
import UIKit

/// Frames decoded off the main thread, waiting for the display link to put them on the layer.
final class PreparedFrames: @unchecked Sendable {
  static let window = 3

  private let lock = NSLock()
  private var order: [Int] = []
  private var images: [Int: CGImage] = [:]

  func image(at index: Int) -> CGImage? { lock.withLock { images[index] } }

  func put(_ image: CGImage, at index: Int) {
    lock.withLock {
      if images.updateValue(image, forKey: index) == nil { order.append(index) }
      while order.count > Self.window { images[order.removeFirst()] = nil }
    }
  }
}

/// The view that owns the animation contents: frames go to `layer.contents` on a display-link tick, never through a `UIImageView`.
final class FrameLayerView: UIView {
  struct Presentation {
    let frames: AnimatedFrames
    let prepared: PreparedFrames
    let rate: Double
    let start: ContinuousClock.Instant
    let limit: Int?
  }

  private final class LinkTarget: NSObject {
    weak var view: FrameLayerView?

    @objc func tick(_ link: CADisplayLink) {
      guard let view else { return link.invalidate() }

      view.tick()
    }
  }

  var windowChanged: (() -> Void)?
  var shape = MediaShape.rectangle {
    didSet { if shape != oldValue { setNeedsLayout() } }
  }
  /// Main-thread writes of `layer.contents`.
  private(set) var framesShown = 0
  var onFrame: ((CGImage) -> Void)?
  private var link: CADisplayLink?
  private var presentation: Presentation?
  private var presentedIndex = -1

  var isPresenting: Bool { link != nil }
  var shownContents: CGImage? {
    guard let contents = layer.contents else { return nil }

    return (contents as! CGImage)
  }

  override init(frame: CGRect) {
    super.init(frame: frame)
    isOpaque = false
    backgroundColor = .clear
    layer.masksToBounds = true
  }

  required init?(coder: NSCoder) { fatalError("init(coder:) is not supported") }

  override func didMoveToWindow() {
    super.didMoveToWindow()
    windowChanged?()
  }

  override func layoutSubviews() {
    super.layoutSubviews()
    layer.cornerRadius = shape.radius(in: bounds.size)
  }

  func show(_ image: CGImage) {
    layer.contents = image
    framesShown += 1
    onFrame?(image)
    TestChange.fire()
  }

  func present(_ presentation: Presentation) {
    stopPresenting()
    self.presentation = presentation
    presentedIndex = -1
    let target = LinkTarget()
    target.view = self
    let link = CADisplayLink(target: target, selector: #selector(LinkTarget.tick(_:)))
    link.add(to: .main, forMode: .common)
    self.link = link
  }

  func stopPresenting() {
    link?.invalidate()
    link = nil
    presentation = nil
  }

  private func tick() {
    guard let presentation else { return }

    let parts = (ContinuousClock.now - presentation.start).components
    let tick = presentation.frames.frameTick(elapsed: Double(parts.seconds) + Double(parts.attoseconds) * 1e-18, rate: presentation.rate)
    if let limit = presentation.limit, tick >= limit { return }

    let index = tick % presentation.frames.frameCount
    guard index != presentedIndex, let image = presentation.prepared.image(at: index) else { return }

    presentedIndex = index
    show(image)
  }
}

struct AnimatedFramesView: UIViewRepresentable {
  struct StreamPlan {
    let stream: FrameStream
    let frames: AnimatedFrames
    let rate: Double
    var repeats: ImageAnimation.Repeats = .forever
    var rest: ImageAnimation.RestFrame = .first
    var grayscale = false

    var interval: TimeInterval { frames.loopDuration(rate: rate) / Double(frames.frameCount) }
    var key: PlanKey { PlanKey(interval: interval, repeats: repeats, rest: rest, grayscale: grayscale) }
    var restIndex: Int { rest == .last ? frames.frameCount - 1 : 0 }
  }

  struct PlanKey: Equatable {
    let interval: TimeInterval
    let repeats: ImageAnimation.Repeats
    let rest: ImageAnimation.RestFrame
    var grayscale = false
  }

  struct Appearance: Equatable {
    let token: UUID
    let grayscale: Bool
  }

  final class Playback {
    var token: UUID?
    var appearance: Appearance?
    var plan: StreamPlan?
    private var task: Task<Void, Never>?
    private var restTask: Task<Void, Never>?
    private var runningKey: PlanKey?
    private var shownRest: RestShown?
    private weak var presenter: FrameLayerView?
    private(set) var finished: PlanKey? { didSet { TestChange.fire() } }

    private struct RestShown: Equatable {
      let token: UUID?
      let rest: ImageAnimation.RestFrame
      let grayscale: Bool
    }

    deinit {
      task?.cancel()
      restTask?.cancel()
    }

    func sync(_ view: FrameLayerView) {
      guard let plan, view.window != nil else { return stop() }
      guard finished != plan.key else { return }
      guard task == nil || runningKey != plan.key else { return }

      stop()
      shownRest = nil
      finished = nil
      runningKey = plan.key
      presenter = view
      task = AnimatedFramesView.play(plan, on: view, finish: { [weak self] key in self?.finished = key })
    }

    func stop() {
      task?.cancel()
      task = nil
      presenter?.stopPresenting()
    }

    func reset() {
      stop()
      restTask?.cancel()
      restTask = nil
      runningKey = nil
      shownRest = nil
      finished = nil
    }

    /// Shows the rest frame once per source and rest choice; the last frame is decoded off the main thread.
    func rest(_ rest: ImageAnimation.RestFrame, of frames: AnimatedFrames, grayscale: Bool, on view: FrameLayerView) {
      plan = nil
      stop()
      finished = nil
      runningKey = nil
      let shown = RestShown(token: token, rest: rest, grayscale: grayscale)
      guard shownRest != shown else { return }

      shownRest = shown
      restTask?.cancel()
      guard rest == .last else {
        restTask = nil
        view.show(AnimatedFramesView.displayed(frames.firstImage, grayscale: grayscale))
        return
      }
      restTask = Task.detached { [weak view] in
        guard let image = frames.image(at: frames.frameCount - 1), !Task.isCancelled else { return }

        let ready = AnimatedFramesView.displayed(image, grayscale: grayscale)
        await MainActor.run { [weak view] in
          if !Task.isCancelled { view?.show(ready) }
        }
      }
    }
  }

  let frames: AnimatedFrames
  let contentMode: ContentMode
  let isPlaying: Bool
  let playbackRate: Double
  var animation = ImageAnimation()
  var rests = false
  var grayscale = false
  var shape = MediaShape.rectangle

  func makeCoordinator() -> Playback { Playback() }

  func makeUIView(context: Context) -> FrameLayerView { FrameLayerView() }

  func updateUIView(_ view: FrameLayerView, context: Context) {
    update(view, playback: context.coordinator)
  }

  func update(_ view: FrameLayerView, playback: Playback) {
    view.layer.contentsGravity = contentMode == .fill ? .resizeAspectFill : .resizeAspect
    view.shape = shape
    view.windowChanged = { [weak playback, weak view] in
      if let playback, let view { playback.sync(view) }
    }
    let appearance = Appearance(token: frames.token, grayscale: grayscale)
    if playback.appearance != appearance {
      playback.appearance = appearance
      playback.token = frames.token
      playback.reset()
      view.show(Self.displayed(frames.firstImage, grayscale: grayscale))
    }
    if rests {
      playback.rest(animation.rest, of: frames, grayscale: grayscale, on: view)
      return
    }
    playback.plan = isPlaying ? StreamPlan(stream: frames.stream, frames: frames, rate: playbackRate, repeats: animation.repeats, rest: animation.rest, grayscale: grayscale) : nil
    playback.sync(view)
  }

  static func dismantleUIView(_ view: FrameLayerView, coordinator: Playback) {
    coordinator.plan = nil
    coordinator.reset()
    view.stopPresenting()
  }

  nonisolated static func displayed(_ image: CGImage, grayscale: Bool) -> CGImage {
    grayscale ? GrayscaleImage.convert(image) : image
  }

  /// Decodes and converts frames off the main thread, the display link of `view` puts them on the layer.
  private static func play(_ plan: StreamPlan, on view: FrameLayerView, finish: @escaping @MainActor (PlanKey) -> Void) -> Task<Void, Never> {
    plan.stream.preparedTask()
    let clock = ContinuousClock()
    let start = clock.now
    let prepared = PreparedFrames()
    let limit: Int? = if case .times(let count) = plan.repeats { max(count, 1) * plan.frames.frameCount } else { nil }
    view.present(FrameLayerView.Presentation(frames: plan.frames, prepared: prepared, rate: plan.rate, start: start, limit: limit))
    return Task.detached { [weak view] in
      func prepare(_ index: Int) {
        guard prepared.image(at: index) == nil, let image = plan.frames.image(at: index) else { return }

        prepared.put(displayed(image, grayscale: plan.grayscale), at: index)
      }
      while !Task.isCancelled {
        let parts = (clock.now - start).components
        let tick = plan.frames.frameTick(elapsed: Double(parts.seconds) + Double(parts.attoseconds) * 1e-18, rate: plan.rate)
        if let limit, tick >= limit {
          let resting = plan.frames.image(at: plan.restIndex).map { displayed($0, grayscale: plan.grayscale) }
          guard !Task.isCancelled else { return }

          await MainActor.run { [weak view] in
            if !Task.isCancelled {
              view?.stopPresenting()
              if let resting { view?.show(resting) }
              finish(plan.key)
            }
          }
          return
        }
        prepare(tick % plan.frames.frameCount)
        guard !Task.isCancelled else { return }

        prepare((tick + 1) % plan.frames.frameCount)
        try? await clock.sleep(until: start + .seconds(plan.frames.frameEnd(tick: tick, rate: plan.rate)))
      }
    }
  }

  func sizeThatFits(_ proposal: ProposedViewSize, uiView: FrameLayerView, context: Context) -> CGSize? {
    proposal.replacingUnspecifiedDimensions()
  }
}
#else
struct AnimatedFramesView: View {
  let frames: AnimatedFrames
  let contentMode: ContentMode
  let isPlaying: Bool
  let playbackRate: Double
  var animation = ImageAnimation()
  var rests = false
  var grayscale = false
  var shape = MediaShape.rectangle

  var body: some View {
    Image(decorative: frames.firstImage, scale: 1).resizable().aspectRatio(contentMode: contentMode)
  }
}
#endif
