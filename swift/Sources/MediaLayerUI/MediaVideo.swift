import MediaLayer
import SwiftUI

public struct MediaVideo<Content: View>: View {
  @Environment(\.mediaContext) private var context
  @State private var state: MediaState = .idle
  private let source: any MediaSource
  private let playing: Bool
  private let muted: Bool
  private let content: (MediaState) -> Content

  public init(
    _ source: some MediaSource,
    playing: Bool = true,
    muted: Bool = false,
    @ViewBuilder content: @escaping (MediaState) -> Content
  ) {
    self.source = source
    self.playing = playing
    self.muted = muted
    self.content = content
  }

  public var body: some View {
    ZStack {
      if let context {
        PlaceholderView(item: source.item, ready: state == .ready, context: context)
        VideoSurface(places: context.places, key: source.url)
      }
      content(state)
    }
    .clipped()
    .task(id: source.url) { await follow() }
    .onAppear { declare(visible: true) }
    .onDisappear { declare(visible: false) }
    .onChange(of: playing) { _, value in context?.places.setPlaying(key: source.url, playing: value) }
    .onChange(of: muted) { _, value in context?.places.setMuted(key: source.url, muted: value) }
  }

  private func declare(visible: Bool) {
    guard let places = context?.places else { return }
    places.visible(source: source.item, visible: visible)
    guard visible else { return }
    places.setPlaying(key: source.url, playing: playing)
    places.setMuted(key: source.url, muted: muted)
  }

  private func follow() async {
    guard let places = context?.places else { return }
    for await value in places.stateStream(key: source.url) { state = value }
  }
}

extension MediaVideo where Content == EmptyView {
  public init(_ source: some MediaSource, playing: Bool = true, muted: Bool = false) {
    self.init(source, playing: playing, muted: muted) { _ in EmptyView() }
  }
}
