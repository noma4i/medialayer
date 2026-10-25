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
  var gif: Data?
  var frames: AnimatedFrames?
}

private struct FramesJob: Equatable {
  let key: ImageKey
  let hasGif: Bool
  let playing: Bool
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

public struct MediaImage<Content: View>: View {
  @ResolvedMediaContext private var context
  @Environment(\.displayScale) private var displayScale
  @State private var loaded: LoadedImage?
  @State private var state: MediaState = .idle
  @State private var failure: ScopedFailure?
  @State private var mount: PlaceMount?
  @State private var mountedItem: MediaItem?
  @State private var mountedContext: ObjectIdentifier?
  @State private var onScreen = false
  @State private var inBackground = false
  private static var coldFadeDuration: Double { 0.22 }
  private let source: any MediaSource
  private let contentMode: ContentMode
  private let playbackRate: Double
  private let content: (MediaState) -> Content

  public init(_ source: some MediaSource, contentMode: ContentMode = .fill, playbackRate: Double = 1, @ViewBuilder content: @escaping (MediaState) -> Content) {
    self.source = source
    self.contentMode = contentMode
    self.playbackRate = playbackRate
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
            }
            if let frames = loaded.flatMap({ $0.key == key ? $0.frames : nil }) {
              AnimatedFramesView(frames: frames, contentMode: contentMode, isPlaying: isPlaying, playbackRate: playbackRate)
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
      .background(OnScreenProbe { screen($0) })
      .task(id: ImageFollow(url: source.url, active: isPlaying)) {
        if isPlaying { await follow() }
      }
      .onChange(of: source.item) { syncClaim() }
      .onReceive(NotificationCenter.default.publisher(for: SystemSignalNames.system.background)) { _ in inBackground = true }
      .onReceive(NotificationCenter.default.publisher(for: SystemSignalNames.system.foreground)) { _ in inBackground = false }
    }
  }

  private var isPlaying: Bool { onScreen && !inBackground }

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
    let next = container.map { LoadedImage(key: key, image: $0.image, gif: $0.type == .gif ? $0.data : nil) }
    if isCold {
      withAnimation(.easeOut(duration: Self.coldFadeDuration)) { loaded = next }
    } else {
      loaded = next
    }
  }

  private func framesJob(for key: ImageKey) -> FramesJob {
    FramesJob(key: key, hasGif: loaded.map { $0.key == key && $0.gif != nil } ?? false, playing: isPlaying)
  }

  private func decodeFrames(_ job: FramesJob) async {
    guard job.playing, job.hasGif, let loaded, loaded.key == job.key, loaded.frames == nil, let data = loaded.gif, let pixels = job.key.pixels else { return }

    let frames = await FrameCache.shared.load(identity: job.key.identity, data: data, maxPixel: Int(max(pixels.width, pixels.height)))
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
  public init(_ source: some MediaSource, contentMode: ContentMode = .fill, playbackRate: Double = 1) {
    self.init(source, contentMode: contentMode, playbackRate: playbackRate) { _ in EmptyView() }
  }
}
