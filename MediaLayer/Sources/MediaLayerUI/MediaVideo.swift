import AVFoundation
import MediaLayer
import SwiftUI

struct VideoEdges {
  private var hasRendered = false

  mutating func advance(to state: MediaState) -> (firstFrame: Bool, ended: Bool) {
    switch state {
    case .idle:
      hasRendered = false
      return (false, false)
    case .rendered:
      defer { hasRendered = true }
      return (!hasRendered, false)
    case .ended:
      return (false, true)
    case .loading, .ready, .failed:
      return (false, false)
    }
  }
}

enum VideoProgress {
  static func fraction(positionMs: UInt64, durationMs: UInt64?) -> Double? {
    guard let durationMs, durationMs > 0 else { return nil }

    return min(Double(positionMs) / Double(durationMs), 1)
  }
}

public struct MediaVideo<Content: View>: View {
  @ResolvedMediaContext private var context
  @State private var state: MediaState = .idle
  @State private var mount: PlaceMount?
  @State private var host: SurfaceHost?
  @State private var onScreen = false
  @State private var inBackground = false
  private let source: any MediaSource
  private let playing: Bool
  private let peeking: Bool
  private let muted: Bool
  private let loop: Bool
  private let group: String?
  private let gravity: AVLayerVideoGravity
  private let onFirstFrame: () -> Void
  private let onEnd: () -> Void
  private let onProgress: ((Double) -> Void)?
  private let content: (MediaState) -> Content

  public init(
    _ source: some MediaSource,
    playing: Bool = true,
    peeking: Bool = false,
    muted: Bool = false,
    loop: Bool = true,
    group: String? = nil,
    gravity: AVLayerVideoGravity = .resizeAspectFill,
    onFirstFrame: @escaping () -> Void = {},
    onEnd: @escaping () -> Void = {},
    onProgress: ((Double) -> Void)? = nil,
    @ViewBuilder content: @escaping (MediaState) -> Content
  ) {
    self.source = source
    self.playing = playing
    self.peeking = peeking
    self.muted = muted
    self.loop = loop
    self.group = group
    self.gravity = gravity
    self.onFirstFrame = onFirstFrame
    self.onEnd = onEnd
    self.onProgress = onProgress
    self.content = content
  }

  public var body: some View {
    ZStack {
      Color.clear
      if active, let context {
        PlaceholderView(item: item, ready: state.isReady, context: context)
        if let host { VideoSurface(host: host, gravity: gravity) }
      }
      if active { content(state) }
    }
    .clipped()
    .task(id: Sampling(url: source.url, active: active, reports: false)) {
      if active { await follow() }
    }
    .task(id: Sampling(url: source.url, active: onScreen && !inBackground, reports: onProgress != nil)) { await sample() }
    .background(OnScreenProbe { value in
      onScreen = value
      declare(visible: value)
    })
    .onReceive(NotificationCenter.default.publisher(for: SystemSignalNames.system.background)) { _ in inBackground = true }
    .onReceive(NotificationCenter.default.publisher(for: SystemSignalNames.system.foreground)) { _ in inBackground = false }
    .onChange(of: onScreen) { _, value in declare(visible: value) }
    .onDisappear { declare(visible: false) }
    .onChange(of: playing) { mount?.update(input()) }
    .onChange(of: group) { mount?.update(input()) }
    .onChange(of: peeking) { mount?.update(input()) }
    .onChange(of: muted) { mount?.update(input()) }
    .id(MountIdentity(item: item, context: context))
  }

  var item: MediaItem {
    var item = source.item
    item.looping = loop
    return item
  }

  private var active: Bool { onScreen && !inBackground }

  private struct Sampling: Equatable {
    let url: String
    let active: Bool
    let reports: Bool
  }

  private func sample() async {
    guard onScreen && !inBackground, let onProgress, let slots = context?.slots else { return }
    let key = await SourceKey.of(source.item)
    guard !Task.isCancelled else { return }
    for await value in slots.progress.stream(for: key) { onProgress(value) }
  }

  private func input() -> PlaceInput {
    .make(visible: true, playing: playing, group: group, peeking: peeking, muted: muted, viewToken: host?.token)
  }

  private func declare(visible: Bool) {
    guard visible else {
      mount?.close()
      mount = nil
      host = nil
      return
    }
    guard mount == nil, let places = context?.places else { return }
    let surface = SurfaceHost(places: places)
    host = surface
    let created = PlaceMount(places: places, item: item) { error in
      Task { @MainActor in state = .failed(detail: String(describing: error)) }
    }
    mount = created
    created.update(input())
  }

  private func follow() async {
    guard let places = context?.places else { return }
    state = .idle
    var edges = VideoEdges()
    for await value in places.stateStream(key: source.url) {
      state = value
      let reached = edges.advance(to: value)
      if reached.firstFrame {
        onFirstFrame()
      }
      if reached.ended { onEnd() }
    }
  }
}

extension MediaVideo where Content == EmptyView {
  public init(
    _ source: some MediaSource,
    playing: Bool = true,
    peeking: Bool = false,
    muted: Bool = false,
    loop: Bool = true,
    group: String? = nil,
    gravity: AVLayerVideoGravity = .resizeAspectFill,
    onFirstFrame: @escaping () -> Void = {},
    onEnd: @escaping () -> Void = {},
    onProgress: ((Double) -> Void)? = nil
  ) {
    self.init(source, playing: playing, peeking: peeking, muted: muted, loop: loop, group: group, gravity: gravity, onFirstFrame: onFirstFrame, onEnd: onEnd, onProgress: onProgress) { _ in EmptyView() }
  }
}
