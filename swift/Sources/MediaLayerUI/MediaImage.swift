import MediaLayer
import Nuke
import SwiftUI

public struct MediaImage<Content: View>: View {
  @Environment(\.mediaContext) private var context
  @State private var image: PlatformImage?
  @State private var state: MediaState = .idle
  private let source: any MediaSource
  private let content: (MediaState) -> Content

  public init(_ source: some MediaSource, @ViewBuilder content: @escaping (MediaState) -> Content) {
    self.source = source
    self.content = content
  }

  public var body: some View {
    ZStack {
      if let context, image == nil {
        PlaceholderView(item: source.item, ready: false, context: context)
      }
      if let image {
        Image(platform: image).resizable().scaledToFill()
      }
      content(state)
    }
    .clipped()
    .task(id: source.url) { await load() }
    .task(id: source.url) { await follow() }
    .onAppear { context?.places.visible(source: source.item, visible: true) }
    .onDisappear { context?.places.visible(source: source.item, visible: false) }
  }

  private func load() async {
    guard let context else { return }
    image = await MediaImagePipeline.load(source, places: context.places, pipeline: context.pipeline)
  }

  private func follow() async {
    guard let places = context?.places else { return }
    for await value in places.stateStream(key: source.url) { state = value }
  }
}

extension MediaImage where Content == EmptyView {
  public init(_ source: some MediaSource) {
    self.init(source) { _ in EmptyView() }
  }
}
