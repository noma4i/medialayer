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
    ImageRequestView(source: source, context: context, points: points, scale: scale, contentMode: contentMode) { key, request in
      PreviewFrame(source: source, context: context, key: key, request: request, contentMode: contentMode)
    }
    .id(PreviewInputs(url: source.url, headers: source.headers, pixelWidth: points.width * scale, pixelHeight: points.height * scale, contentMode: contentMode))
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
  let pixelWidth: CGFloat?
  let pixelHeight: CGFloat?
  let contentMode: ContentMode
  let context: ObjectIdentifier
}

private final class PreparedImage: ObservableObject {
  let key: ImageKey
  let request: ImageRequest?

  init(source: any MediaSource, context: MediaContext, points: CGSize, scale: CGFloat, contentMode: ContentMode) {
    (key, request) = MediaImagePipeline.prepare(places: context.places, for: source, points: points, scale: scale, contentMode: contentMode)
  }
}

private struct ImageRequestView<Content: View>: View {
  @StateObject private var prepared: PreparedImage
  private let content: (ImageKey, ImageRequest?) -> Content

  init(source: any MediaSource, context: MediaContext, points: CGSize, scale: CGFloat, contentMode: ContentMode, @ViewBuilder content: @escaping (ImageKey, ImageRequest?) -> Content) {
    _prepared = StateObject(wrappedValue: PreparedImage(source: source, context: context, points: points, scale: scale, contentMode: contentMode))
    self.content = content
  }

  var body: some View { content(prepared.key, prepared.request) }
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
  @State private var loaded: LoadedImage?
  @State private var state: MediaState = .idle
  @State private var failure: ScopedFailure?
  @State private var claim = ClaimedMount()
  @State private var mountedItem: MediaItem?
  @State private var mountedContext: ObjectIdentifier?
  @State private var onScreen = false
  @State private var inBackground = false
  private static var coldFadeDuration: Double { 0.22 }
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
    self.playbackRate = playbackRate
    self.animation = animation
    self.content = content
  }

  public var body: some View {
    mounted.onChange(of: context.map { ObjectIdentifier($0.places) }) { syncClaim() }
  }

  @ViewBuilder private var mounted: some View {
    if let context {
      GeometryReader { geometry in
        let pixels = MediaImagePipeline.decodeSize(places: context.places, for: source, points: geometry.size, scale: displayScale, contentMode: contentMode)
        ImageRequestView(source: source, context: context, points: geometry.size, scale: displayScale, contentMode: contentMode) { key, request in
          let shown = shownImage(for: key, request: request)
          ZStack {
            if isPlaying && shown == nil {
              PlaceholderView(item: source.item, ready: false, context: context)
              if let preview, loaded.flatMap({ $0.key == key ? $0.frames : nil }) == nil {
                PreviewImage(source: preview, context: context, points: geometry.size, scale: displayScale, contentMode: contentMode)
              }
            }
            if let frames = loaded.flatMap({ $0.key == key ? $0.frames : nil }) {
              AnimatedFramesView(frames: frames, contentMode: contentMode, isPlaying: isPlaying, playbackRate: playbackRate, animation: animation, rests: animation.rests(reduceMotion: reduceMotion))
            } else if let shown {
              Image(platform: shown).resizable().aspectRatio(contentMode: contentMode).transition(.opacity)
            }
            if isPlaying { content(failure?.of(source.item) ?? state) }
          }
          .frame(width: geometry.size.width, height: geometry.size.height)
          .clipped()
          .task(id: ImageLoad(key: key, active: isPlaying)) {
            if isPlaying { await load(key: key, request: request) }
          }
          .task(id: framesJob(for: key)) { [job = framesJob(for: key)] in await decodeFrames(job) }
        }
        .id(ImageInputs(url: source.url, headers: source.headers, width: source.size?.width, height: source.size?.height, pixelWidth: pixels?.width, pixelHeight: pixels?.height, contentMode: contentMode, context: ObjectIdentifier(context)))
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

  private var mount: PlaceMount? {
    get { claim.mount }
    nonmutating set { claim.mount = newValue }
  }

  private var isPlaying: Bool { onScreen && !inBackground }
  private var reduceMotion: Bool { reduceMotionOverride ?? systemReduceMotion }

  private struct ImageLoad: Equatable {
    let key: ImageKey
    let active: Bool
  }

  private struct ImageFollow: Equatable {
    let url: String
    let active: Bool
  }

  private func screen(_ visible: Bool) {
    onScreen = visible
    syncClaim()
  }

  private func syncClaim() {
    let item = source.item
    let places = context?.places
    let current = places.map { ObjectIdentifier($0) }
    if let mounted = mountedItem, !onScreen || mounted != item || mountedContext != current {
      mount?.close()
      mount = nil
      mountedItem = nil
      mountedContext = nil
    }
    guard let places else { return }
    if onScreen, mount == nil {
      let created = PlaceMount(places: places, item: item) { error in
        Task { @MainActor in failure = ScopedFailure(item: item, state: .failed(detail: String(describing: error))) }
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
    let isCold = MediaImagePipeline.cachedContainer(for: request, in: context.pipeline) == nil
    let report: (@Sendable (MediaState) -> Void)? = mount.map { mount in { state in mount.report(state) } }
    let container = await MediaImagePipeline.load(source, request: request, pipeline: context.pipeline, report: report)
    let next = container.map { LoadedImage(key: key, image: $0.image, animated: $0.type == .gif || $0.type == .webp ? $0.data : nil) }
    if isCold, preview == nil {
      withAnimation(.easeOut(duration: Self.coldFadeDuration)) { loaded = next }
    } else {
      loaded = next
    }
  }

  private func framesJob(for key: ImageKey) -> FramesJob {
    let resting = animation.rests(reduceMotion: reduceMotion)
    return FramesJob(key: key, hasAnimation: loaded.map { $0.key == key && $0.animated != nil } ?? false, needed: isPlaying && (!resting || animation.rest == .last))
  }

  private func decodeFrames(_ job: FramesJob) async {
    guard job.needed, job.hasAnimation, let loaded, loaded.key == job.key, loaded.frames == nil, let data = loaded.animated, let pixels = job.key.pixels else { return }

    let frames = await FrameCache.shared.load(identity: job.key.identity, data: data, maxPixel: Int(max(pixels.width, pixels.height)), admit: FrameCache.admit(from: context?.places as? MediaLayer))
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

  func close() {
    mount?.close()
    mount = nil
  }
}
