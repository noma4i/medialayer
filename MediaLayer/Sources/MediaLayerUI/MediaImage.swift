import MediaLayer
import Nuke
import SwiftUI

struct ImageKey: Equatable {
  let identity: String
  let pixels: CGSize?
  let contentMode: ContentMode
}

private struct LoadedImage {
  let key: ImageKey
  let image: PlatformImage
  var animated: Data?
  var frames: AnimatedFrames?
}

private struct PreviewFrame: View {
  let source: any MediaSource
  let context: MediaContext
  let key: ImageKey
  let request: ImageRequest?
  let contentMode: ContentMode
  @State private var loaded: PlatformImage?

  var body: some View {
    Color.clear.overlay {
      if let image = loaded ?? cached {
        Image(platform: image).resizable().aspectRatio(contentMode: contentMode)
      }
    }
    .task(id: key) {
      guard loaded == nil, key.pixels != nil, let request else { return }
      loaded = await MediaImagePipeline.load(source, request: request, pipeline: context.pipeline)?.image
    }
  }

  private var cached: PlatformImage? {
    guard key.pixels != nil, let request else { return nil }
    return MediaImagePipeline.cachedImage(for: request, in: context.pipeline)
  }
}

private struct PreviewImage: View {
  let source: any MediaSource
  let context: MediaContext
  let points: CGSize
  let scale: CGFloat
  let contentMode: ContentMode

  var body: some View {
    ImageRequestView(source: source, context: context, points: points, scale: scale, contentMode: contentMode, waiting: { Color.clear }) { key, request in
      PreviewFrame(source: source, context: context, key: key, request: request, contentMode: contentMode)
    }
    .id(ImageInputs(url: source.url, headers: source.headers, width: source.size?.width, height: source.size?.height, contentMode: contentMode, context: ObjectIdentifier(context)))
  }
}

private struct PreviewInputs: Hashable {
  let url: String
  let headers: [String: String]
  let pixelWidth: CGFloat
  let pixelHeight: CGFloat
  let contentMode: ContentMode
}

private struct FramesJob: Equatable {
  let key: ImageKey
  let hasAnimation: Bool
  let needed: Bool
}

private struct ImageInputs: Hashable {
  let url: String
  let headers: [String: String]
  let width: Int?
  let height: Int?
  let contentMode: ContentMode
  let context: ObjectIdentifier
}

private final class PreparedImage: ObservableObject {
  @Published private(set) var value: (key: ImageKey, request: ImageRequest?)?

  init(source: any MediaSource, context: MediaContext, points: CGSize, scale: CGFloat, contentMode: ContentMode) {
    value = MediaImagePipeline.cachedPreparation(places: context.places, for: source, points: points, scale: scale, contentMode: contentMode)
  }

  @MainActor
  func load(source: any MediaSource, context: MediaContext, points: CGSize, scale: CGFloat, contentMode: ContentMode) async {
    let next = await MediaImagePipeline.prepareOffMain(places: context.places, for: source, points: points, scale: scale, contentMode: contentMode)
    guard !Task.isCancelled else { return }
    if value?.key != next.key { value = next }
  }
}

private struct ImageRequestView<Content: View, Waiting: View>: View {
  @StateObject private var prepared: PreparedImage
  private let source: any MediaSource
  private let context: MediaContext
  private let points: CGSize
  private let scale: CGFloat
  private let contentMode: ContentMode
  private let content: (ImageKey, ImageRequest?) -> Content
  private let waiting: () -> Waiting

  init(source: any MediaSource, context: MediaContext, points: CGSize, scale: CGFloat, contentMode: ContentMode, @ViewBuilder waiting: @escaping () -> Waiting, @ViewBuilder content: @escaping (ImageKey, ImageRequest?) -> Content) {
    _prepared = StateObject(wrappedValue: PreparedImage(source: source, context: context, points: points, scale: scale, contentMode: contentMode))
    self.source = source
    self.context = context
    self.points = points
    self.scale = scale
    self.contentMode = contentMode
    self.content = content
    self.waiting = waiting
  }

  var body: some View {
    Group {
      if let value = prepared.value { content(value.key, value.request) }
      else { waiting() }
    }
    .task(id: PreviewInputs(url: source.url, headers: source.headers, pixelWidth: points.width * scale, pixelHeight: points.height * scale, contentMode: contentMode)) {
      await prepared.load(source: source, context: context, points: points, scale: scale, contentMode: contentMode)
    }
  }
}

private struct ReduceMotionOverrideKey: EnvironmentKey {
  static let defaultValue: Bool? = nil
}

extension EnvironmentValues {
  /// Replaces the system "reduce motion" setting for the views below; the system value is read when it is nil.
  var mediaReduceMotionOverride: Bool? {
    get { self[ReduceMotionOverrideKey.self] }
    set { self[ReduceMotionOverrideKey.self] = newValue }
  }
}

public struct MediaImage<Content: View>: View {
  @ResolvedMediaContext private var context
  @Environment(\.displayScale) private var displayScale
  @Environment(\.accessibilityReduceMotion) private var systemReduceMotion
  @Environment(\.mediaReduceMotionOverride) private var reduceMotionOverride
  @Environment(\.mediaGrayscale) private var grayscale
  @Environment(\.mediaShape) private var shape
  @State private var loaded: LoadedImage?
  @State private var state: MediaState = .idle
  @State private var loadTurn: UInt64 = 0
  @State private var failure: ScopedFailure?
  @State private var claim = ClaimedMount()
  @State private var mountedItem: MediaItem?
  @State private var mountedContext: ObjectIdentifier?
  @State private var onScreen = false
  @State private var inBackground = false
  private let source: any MediaSource
  private let preview: (any MediaSource)?
  private let contentMode: ContentMode
  private let playbackRate: Double
  private let animation: ImageAnimation
  private let content: (MediaState) -> Content

  public init(_ source: some MediaSource, contentMode: ContentMode = .fill, playbackRate: Double = 1, animation: ImageAnimation = ImageAnimation(), @ViewBuilder content: @escaping (MediaState) -> Content) {
    self.init(shown: source, preview: nil, contentMode: contentMode, playbackRate: playbackRate, animation: animation, content: content)
  }

  public init(_ source: some MediaSource, preview: (some MediaSource)?, contentMode: ContentMode = .fill, playbackRate: Double = 1, animation: ImageAnimation = ImageAnimation(), @ViewBuilder content: @escaping (MediaState) -> Content) {
    self.init(shown: source, preview: preview, contentMode: contentMode, playbackRate: playbackRate, animation: animation, content: content)
  }

  private init(shown source: any MediaSource, preview: (any MediaSource)?, contentMode: ContentMode, playbackRate: Double, animation: ImageAnimation, content: @escaping (MediaState) -> Content) {
    self.source = source
    self.preview = preview
    self.contentMode = contentMode
    self.playbackRate = playbackRateOf(playbackRate)
    self.animation = animation
    self.content = content
  }

  public var body: some View {
    mounted.onChange(of: context.map { ObjectIdentifier($0.places) }) { syncClaim() }
  }

  @ViewBuilder private var mounted: some View {
    if let context {
      GeometryReader { geometry in
        ImageRequestView(source: source, context: context, points: geometry.size, scale: displayScale, contentMode: contentMode, waiting: { waitingFrame(context: context, size: geometry.size) }) { key, request in
          let shown = shownImage(for: key, request: request)
          ZStack {
            if ImagePass.drawsPlaceholder(playing: isPlaying, shown: shown != nil) {
              PlaceholderView(item: source.item, ready: false, context: context)
              if let preview, loaded.flatMap({ $0.key == key ? $0.frames : nil }) == nil {
                PreviewImage(source: preview, context: context, points: geometry.size, scale: displayScale, contentMode: contentMode)
              }
            }
            if let frames = loaded.flatMap({ $0.key == key ? $0.frames : nil }) {
              AnimatedFramesView(frames: frames, contentMode: contentMode, isPlaying: isPlaying, playbackRate: playbackRate, animation: animation, rests: animation.rests(reduceMotion: reduceMotion), grayscale: grayscale, shape: shape)
            } else if let shown {
              still(shown)
            }
            if isPlaying { content(ImagePass.contentState(shown: shown != nil, state: state, failure: failure?.of(source.item))) }
          }
          .frame(width: geometry.size.width, height: geometry.size.height)
          .clipped()
          .task(id: ImageLoad(key: key, active: isPlaying, turn: loadTurn)) {
            if isPlaying { await load(key: key, request: request) }
          }
          .task(id: framesJob(for: key)) { [job = framesJob(for: key)] in await decodeFrames(job) }
        }
        .id(ImageInputs(url: source.url, headers: source.headers, width: source.size?.width, height: source.size?.height, contentMode: contentMode, context: ObjectIdentifier(context)))
      }
      .background(OnScreenProbe(onChange: { screen($0) }, onDismantle: { [claim] in claim.close() }))
      .task(id: ImageFollow(url: source.url, active: isPlaying)) {
        if isPlaying { await follow() }
      }
      .onChange(of: source.item) { syncClaim() }
      .onReceive(NotificationCenter.default.publisher(for: SystemSignalNames.system.background)) { _ in inBackground = true }
      .onReceive(NotificationCenter.default.publisher(for: SystemSignalNames.system.foreground)) { _ in inBackground = false }
    }
  }

  /// The pass before the new source has its request: the placeholder and the preview stay drawn, so a source switch never shows an empty frame.
  @ViewBuilder private func waitingFrame(context: MediaContext, size: CGSize) -> some View {
    ZStack {
      if ImagePass.drawsPlaceholder(playing: isPlaying, shown: false) {
        PlaceholderView(item: source.item, ready: false, context: context)
        if let preview { PreviewImage(source: preview, context: context, points: size, scale: displayScale, contentMode: contentMode) }
      }
    }
    .frame(width: size.width, height: size.height)
    .clipped()
  }

  @ViewBuilder private func still(_ image: PlatformImage) -> some View {
    #if canImport(UIKit)
    if grayscale || shape != .rectangle {
      StillLayerView(image: image, contentMode: contentMode, grayscale: grayscale, shape: shape)
    } else {
      Image(platform: image).resizable().aspectRatio(contentMode: contentMode)
    }
    #else
    Image(platform: image).resizable().aspectRatio(contentMode: contentMode)
    #endif
  }

  private var mount: PlaceMount? {
    get { claim.mount }
    nonmutating set { claim.mount = newValue }
  }

  private var isPlaying: Bool { onScreen && !inBackground }
  private var reduceMotion: Bool { reduceMotionOverride ?? systemReduceMotion }

  private struct ImageLoad: Equatable {
    let key: ImageKey
    let active: Bool
    let turn: UInt64
  }

  private struct ImageFollow: Equatable {
    let url: String
    let active: Bool
  }

  private func screen(_ visible: Bool) {
    MediaTrace.emit("view", "onScreen \(MediaTrace.tail(source.url)) \(visible)")
    onScreen = visible
    syncClaim()
  }

  private func syncClaim() {
    let item = source.item
    let places = context?.places
    let current = places.map { ObjectIdentifier($0) }
    if let mounted = mountedItem, !onScreen || mounted != item || mountedContext != current {
      claim.close()
      mountedItem = nil
      mountedContext = nil
    }
    guard let places else { return }
    if onScreen, mount == nil {
      let token = claim.gate.open()
      let relay = PlaybackRelay(
        receive: { [gate = claim.gate] playback in gate.deliver(token) { loadTurn = playback.attempt } },
        failure: { _ in }
      )
      let created = PlaceMount(places: places, item: item, playback: relay) { [gate = claim.gate] error in
        gate.deliver(token) { failure = ScopedFailure(item: item, state: .failed(detail: String(describing: error))) }
      }
      created.update(.make(visible: true))
      mount = created
      mountedItem = item
      mountedContext = current
    }
  }

  private func shownImage(for key: ImageKey, request: ImageRequest?) -> PlatformImage? {
    if let loaded, loaded.key == key { return loaded.image }
    guard let context, key.pixels != nil, let request else { return nil }
    return MediaImagePipeline.cachedImage(for: request, in: context.pipeline)
  }

  private func load(key: ImageKey, request: ImageRequest?) async {
    guard loaded?.key != key, let context, key.pixels != nil, let request else { return }
    let tail = MediaTrace.tail(source.url)
    MediaTrace.emit("image", "load \(tail) start")
    let report: (@Sendable (MediaState) -> Void)? = mount.map { mount in { @Sendable state in mount.report(state) } }
    let container = await MediaImagePipeline.load(source, request: request, pipeline: context.pipeline, report: report)
    MediaTrace.emit("image", "load \(tail) \(container == nil ? "failed" : "done")")
    let next = container.map { LoadedImage(key: key, image: $0.image, animated: $0.type == .gif || $0.type == .webp ? $0.data : nil) }
    loaded = next
  }

  private func framesJob(for key: ImageKey) -> FramesJob {
    let resting = animation.rests(reduceMotion: reduceMotion)
    return FramesJob(key: key, hasAnimation: loaded.map { $0.key == key && $0.animated != nil } ?? false, needed: isPlaying && (!resting || animation.rest == .last))
  }

  private func decodeFrames(_ job: FramesJob) async {
    guard job.needed, job.hasAnimation, let loaded, loaded.key == job.key, loaded.frames == nil, let data = loaded.animated, let pixels = job.key.pixels else { return }

    let cache = context.map { ImageMemory.frames(for: $0.places) } ?? FrameCache.shared
    let frames = await cache.load(identity: job.key.identity, data: data, maxPixel: Int(max(pixels.width, pixels.height)), admit: FrameCache.admit(from: context?.places as? MediaLayer))
    guard !Task.isCancelled, self.loaded?.key == job.key else { return }

    self.loaded?.frames = frames
  }

  private func follow() async {
    guard let context else { return }
    let places = context.places
    let stats = context.stats
    let url = source.url
    stats.recordLater(work: .observe)
    defer { stats.recordLater(work: .unobserve) }
    let stream = await Task.detached { places.stateStream(key: url) }.value
    for await value in stream { state = value }
  }
}

/// What one render pass of `MediaImage` draws: the image and a loading state or a placeholder never share a pass.
enum ImagePass {
  static func drawsPlaceholder(playing: Bool, shown: Bool) -> Bool { playing && !shown }

  static func contentState(shown: Bool, state: MediaState, failure: MediaState?) -> MediaState {
    if let failure { return failure }
    guard shown else { return state }
    switch state {
    case .idle, .loading: return .ready
    default: return state
    }
  }
}

extension MediaImage where Content == EmptyView {
  public init(_ source: some MediaSource, contentMode: ContentMode = .fill, playbackRate: Double = 1, animation: ImageAnimation = ImageAnimation()) {
    self.init(source, contentMode: contentMode, playbackRate: playbackRate, animation: animation) { _ in EmptyView() }
  }

  public init(_ source: some MediaSource, preview: (some MediaSource)?, contentMode: ContentMode = .fill, playbackRate: Double = 1, animation: ImageAnimation = ImageAnimation()) {
    self.init(source, preview: preview, contentMode: contentMode, playbackRate: playbackRate, animation: animation) { _ in EmptyView() }
  }
}

/// The place claim of an image. A plain object, so the teardown of the view releases it without writing view state.
final class ClaimedMount {
  var mount: PlaceMount?
  let gate = DeliveryGate()

  func close() {
    gate.close()
    mount?.close()
    mount = nil
  }
}
