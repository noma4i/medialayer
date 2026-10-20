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

public struct MediaImage<Content: View>: View {
  @Environment(\.mediaContext) private var context
  @Environment(\.displayScale) private var displayScale
  @State private var loaded: LoadedImage?
  @State private var state: MediaState = .idle
  @State private var owner = UUID()
  @State private var claimedURL: String?
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
    if let context {
      GeometryReader { geometry in
        let key = imageKey(points: geometry.size, places: context.places)
        let shown = shownImage(for: key, points: geometry.size)
        ZStack {
          if shown == nil {
            PlaceholderView(item: source.item, ready: false, context: context)
          }
          if let frames = loaded.flatMap({ $0.key == key ? $0.frames : nil }) {
            AnimatedFramesView(frames: frames, contentMode: contentMode, isPlaying: isPlaying, playbackRate: playbackRate)
          } else if let shown {
            Image(platform: shown).resizable().aspectRatio(contentMode: contentMode).transition(.opacity)
          }
          content(state)
        }
        .frame(width: geometry.size.width, height: geometry.size.height)
        .clipped()
        .task(id: key) { await load(key: key, points: geometry.size) }
        .task(id: framesJob(for: key)) { [job = framesJob(for: key)] in await decodeFrames(job) }
      }
      .background(OnScreenProbe { screen($0) })
      .task(id: source.url) { await follow() }
      .onChange(of: source.url) { syncClaim() }
      .onReceive(NotificationCenter.default.publisher(for: SystemSignalNames.system.background)) { _ in inBackground = true }
      .onReceive(NotificationCenter.default.publisher(for: SystemSignalNames.system.foreground)) { _ in inBackground = false }
    }
  }

  private var isPlaying: Bool { onScreen && !inBackground }

  private func screen(_ visible: Bool) {
    onScreen = visible
    syncClaim()
  }

  private func syncClaim() {
    guard let places = context?.places else { return }
    let claims = KeyClaims.of(places)
    if let claimed = claimedURL, !onScreen || claimed != source.url {
      claims.release(owner: owner, key: claimed)
      claimedURL = nil
    }
    if onScreen, claimedURL == nil {
      claims.visible(owner: owner, source: source.item, visible: true)
      claimedURL = source.url
    }
  }

  private func imageKey(points: CGSize, places: MediaPlaces) -> ImageKey {
    ImageKey(identity: places.cacheKey(url: source.url, headers: source.headers), pixels: MediaImagePipeline.decodeSize(for: source, points: points, scale: displayScale), contentMode: contentMode)
  }

  private func request(points: CGSize) -> ImageRequest? {
    guard let context else { return nil }
    return MediaImagePipeline.request(places: context.places, for: source, points: points, scale: displayScale, contentMode: contentMode)
  }

  private func shownImage(for key: ImageKey, points: CGSize) -> PlatformImage? {
    if let loaded, loaded.key == key { return loaded.image }
    guard let context, key.pixels != nil, let request = request(points: points) else { return nil }
    return MediaImagePipeline.cachedImage(for: request, in: context.pipeline)
  }

  private func load(key: ImageKey, points: CGSize) async {
    guard let context, key.pixels != nil, let request = request(points: points) else { return }
    let container: ImageContainer?
    let isCold: Bool
    if let hit = MediaImagePipeline.cachedContainer(for: request, in: context.pipeline) {
      context.places.itemState(key: source.url, state: .ready)
      container = hit
      isCold = false
    } else {
      container = await MediaImagePipeline.load(source, request: request, places: context.places, pipeline: context.pipeline)
      isCold = true
    }
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
    guard let places = context?.places else { return }
    for await value in places.stateStream(key: source.url) { state = value }
  }
}

extension MediaImage where Content == EmptyView {
  public init(_ source: some MediaSource, contentMode: ContentMode = .fill, playbackRate: Double = 1) {
    self.init(source, contentMode: contentMode, playbackRate: playbackRate) { _ in EmptyView() }
  }
}
