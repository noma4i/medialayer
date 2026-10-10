import MediaLayer
import Nuke
import SwiftUI

struct ImageKey: Equatable {
  let url: String
  let pixels: CGSize?
  let contentMode: ContentMode
}

private struct LoadedImage {
  let key: ImageKey
  let image: PlatformImage
  var frames: AnimatedFrames?
}

public struct MediaImage<Content: View>: View {
  @Environment(\.mediaContext) private var context
  @Environment(\.displayScale) private var displayScale
  @State private var loaded: LoadedImage?
  @State private var state: MediaState = .idle
  @State private var isVisible = false
  private let source: any MediaSource
  private let contentMode: ContentMode
  private let content: (MediaState) -> Content

  public init(_ source: some MediaSource, contentMode: ContentMode = .fill, @ViewBuilder content: @escaping (MediaState) -> Content) {
    self.source = source
    self.contentMode = contentMode
    self.content = content
  }

  public var body: some View {
    GeometryReader { geometry in
      let key = imageKey(points: geometry.size)
      let shown = shownImage(for: key, points: geometry.size)
      ZStack {
        if let context, shown == nil {
          PlaceholderView(item: source.item, ready: false, context: context)
        }
        if let frames = loaded.flatMap({ $0.key == key ? $0.frames : nil }) {
          AnimatedFramesView(frames: frames, contentMode: contentMode, isPlaying: isVisible)
        } else if let shown {
          Image(platform: shown).resizable().aspectRatio(contentMode: contentMode)
        }
        content(state)
      }
      .frame(width: geometry.size.width, height: geometry.size.height)
      .clipped()
      .task(id: key) { await load(key: key, points: geometry.size) }
    }
    .task(id: source.url) { await follow() }
    .onAppear {
      isVisible = true
      context?.places.visible(source: source.item, visible: true)
    }
    .onDisappear {
      isVisible = false
      context?.places.visible(source: source.item, visible: false)
    }
  }

  private func imageKey(points: CGSize) -> ImageKey {
    ImageKey(url: source.url, pixels: MediaImagePipeline.decodeSize(for: source, points: points, scale: displayScale), contentMode: contentMode)
  }

  private func request(points: CGSize) -> ImageRequest? {
    MediaImagePipeline.request(for: source, points: points, scale: displayScale, contentMode: contentMode)
  }

  private func shownImage(for key: ImageKey, points: CGSize) -> PlatformImage? {
    if let loaded, loaded.key == key { return loaded.image }
    guard let context, key.pixels != nil, let request = request(points: points) else { return nil }
    return MediaImagePipeline.cachedImage(for: request, in: context.pipeline)
  }

  private func load(key: ImageKey, points: CGSize) async {
    guard let context, key.pixels != nil, let request = request(points: points) else { return }
    let container: ImageContainer?
    if let hit = MediaImagePipeline.cachedContainer(for: request, in: context.pipeline) {
      context.places.itemState(key: source.url, state: .ready)
      container = hit
    } else {
      container = await MediaImagePipeline.load(source, request: request, places: context.places, pipeline: context.pipeline)
    }
    loaded = container.map { LoadedImage(key: key, image: $0.image) }
    guard let container, container.type == .gif, let data = container.data, let pixels = key.pixels else { return }

    let frames = await AnimatedFrames.load(data, maxPixel: Int(max(pixels.width, pixels.height)))
    guard !Task.isCancelled, loaded?.key == key else { return }

    loaded?.frames = frames
  }

  private func follow() async {
    guard let places = context?.places else { return }
    for await value in places.stateStream(key: source.url) { state = value }
  }
}

extension MediaImage where Content == EmptyView {
  public init(_ source: some MediaSource, contentMode: ContentMode = .fill) {
    self.init(source, contentMode: contentMode) { _ in EmptyView() }
  }
}
