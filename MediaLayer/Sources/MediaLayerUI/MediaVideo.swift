import AVFoundation
import MediaLayer
import SwiftUI

struct VideoEdges {
  private var hasRendered = false

  mutating func advance(to state: MediaState) -> (firstFrame: Bool, ended: Bool) {
    switch state {
    case .idle, .loading:
      hasRendered = false
      return (false, false)
    case .rendered:
      defer { hasRendered = true }
      return (!hasRendered, false)
    case .ended:
      return (false, true)
    case .ready, .failed:
      return (false, false)
    }
  }
}

enum VideoProgress {
  static let interval: Duration = .milliseconds(100)

  static func fraction(positionMs: UInt64, durationMs: UInt64?) -> Double? {
    guard let durationMs, durationMs > 0 else { return nil }

    return min(Double(positionMs) / Double(durationMs), 1)
  }
}

public struct MediaVideo<Content: View>: View {
  @Environment(\.mediaContext) private var context
  @State private var state: MediaState = .idle
  @State private var owner = UUID()
  private let source: any MediaSource
  private let playing: Bool
  private let peeking: Bool
  private let muted: Bool
  private let loop: Bool
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
    self.gravity = gravity
    self.onFirstFrame = onFirstFrame
    self.onEnd = onEnd
    self.onProgress = onProgress
    self.content = content
  }

  public var body: some View {
    ZStack {
      if let context {
        PlaceholderView(item: item, ready: state.isReady, context: context)
        VideoSurface(places: context.places, key: source.url, owner: owner, isPlaying: playing, gravity: gravity)
      }
      content(state)
    }
    .clipped()
    .task(id: source.url) { await follow() }
    .task(id: Sampling(url: source.url, playing: playing)) { await sample() }
    .onAppear { declare(visible: true) }
    .onDisappear { declare(visible: false) }
    .onChange(of: playing) { _, value in context.map { KeyClaims.of($0.places).setPlaying(owner: owner, key: source.url, playing: value) } }
    .onChange(of: peeking) { _, value in context.map { KeyClaims.of($0.places).setPeeking(owner: owner, key: source.url, peeking: value) } }
    .onChange(of: muted) { _, value in context.map { KeyClaims.of($0.places).setMuted(owner: owner, key: source.url, muted: value) } }
  }

  var item: MediaItem {
    var item = source.item
    item.looping = loop
    return item
  }

  private struct Sampling: Equatable {
    let url: String
    let playing: Bool
  }

  private func sample() async {
    guard let onProgress, let slots = context?.slots else { return }

    repeat {
      if let sample = slots.slot(for: source.url)?.sample(),
         let fraction = VideoProgress.fraction(positionMs: sample.positionMs, durationMs: sample.durationMs) {
        onProgress(fraction)
      }
      guard playing else { return }

      try? await Task.sleep(for: VideoProgress.interval)
    } while !Task.isCancelled
  }

  private func declare(visible: Bool) {
    guard let places = context?.places else { return }
    let claims = KeyClaims.of(places)
    guard visible else { return claims.release(owner: owner, key: source.url) }
    claims.visible(owner: owner, source: item, visible: true)
    claims.setMuted(owner: owner, key: source.url, muted: muted)
    claims.setPlaying(owner: owner, key: source.url, playing: playing)
    claims.setPeeking(owner: owner, key: source.url, peeking: peeking)
  }

  private func follow() async {
    guard let places = context?.places else { return }
    var edges = VideoEdges()
    for await value in places.stateStream(key: source.url) {
      state = value
      let reached = edges.advance(to: value)
      if reached.firstFrame { onFirstFrame() }
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
    gravity: AVLayerVideoGravity = .resizeAspectFill,
    onFirstFrame: @escaping () -> Void = {},
    onEnd: @escaping () -> Void = {},
    onProgress: ((Double) -> Void)? = nil
  ) {
    self.init(source, playing: playing, peeking: peeking, muted: muted, loop: loop, gravity: gravity, onFirstFrame: onFirstFrame, onEnd: onEnd, onProgress: onProgress) { _ in EmptyView() }
  }
}
