import MediaLayer
import Observation
import SwiftUI

@MainActor
@Observable
public final class AudioPlayback {
  public private(set) var isPlaying: Bool
  public private(set) var progress = 0.0
  public private(set) var rate: Float
  public private(set) var state: MediaState = .idle
  @ObservationIgnored private let item: MediaItem
  @ObservationIgnored private(set) var group: String?
  @ObservationIgnored private var muted: Bool
  @ObservationIgnored private let places: MediaPlaces
  @ObservationIgnored private var mount: PlaceMount?

  init(item: MediaItem, playing: Bool, muted: Bool = false, rate: Float = 1, group: String? = nil, places: MediaPlaces) {
    self.item = item
    self.group = group
    self.muted = muted
    self.rate = rate
    self.places = places
    isPlaying = playing
  }

  func mountPlace() {
    guard mount == nil else { return }
    mount = PlaceMount(places: places, item: item) { [weak self] error in
      Task { @MainActor in self?.follow(.failed(detail: String(describing: error))) }
    }
    push()
  }

  func unmountPlace() {
    mount?.close()
    mount = nil
  }

  func follow(muted value: Bool) {
    muted = value
    push()
  }

  func follow(group value: String?) {
    group = value
    push()
  }

  private func push() {
    mount?.update(.make(visible: true, playing: isPlaying, group: group, muted: muted, rate: rate))
  }

  public func toggle() {
    setPlaying(!isPlaying)
  }

  func follow(playing value: Bool) {
    setPlaying(value)
  }

  public func setRate(_ value: Float) {
    rate = value
    push()
  }

  public func seek(to fraction: Double) {
    guard fraction.isFinite else { return }
    let clamped = min(max(fraction, 0), 1)
    refresh(progress: clamped)
    mount?.seek(clamped)
  }

  func refresh(progress value: Double) {
    if progress != value { progress = value }
  }

  public var isSounding: Bool {
    switch state {
    case .ready, .rendered: isPlaying
    case .idle, .loading, .ended, .failed: false
    }
  }

  public var isBuffering: Bool {
    switch state {
    case .idle, .loading: isPlaying
    case .ready, .rendered, .ended, .failed: false
    }
  }

  @discardableResult
  func follow(_ value: MediaState) -> Bool {
    let isNewEnd = value == .ended && state != .ended
    if state != value { state = value }
    guard value == .ended else { return false }
    if isPlaying { setPlaying(false) }
    guard isNewEnd else { return false }
    refresh(progress: 0)
    mount?.seek(0)
    return true
  }

  private func setPlaying(_ value: Bool) {
    isPlaying = value
    push()
  }
}

public struct MediaAudio<Content: View>: View {

  @ResolvedMediaContext private var context
  private let source: any MediaSource
  private let playing: Bool
  private let muted: Bool
  private let rate: Float
  private let group: String?
  private let onEnd: () -> Void
  private let content: (AudioPlayback) -> Content

  public init(
    _ source: some MediaSource,
    playing: Bool = false,
    muted: Bool = false,
    rate: Float = 1,
    group: String? = nil,
    onEnd: @escaping () -> Void = {},
    @ViewBuilder content: @escaping (AudioPlayback) -> Content
  ) {
    self.source = source
    self.playing = playing
    self.muted = muted
    self.rate = rate
    self.group = group
    self.onEnd = onEnd
    self.content = content
  }

  public var body: some View {
    if let context {
      Placed(source: source, playing: playing, muted: muted, rate: rate, group: group, context: context, onEnd: onEnd, content: content)
        .id(MountIdentity(item: source.item, context: context))
    }
  }

  private struct Placed: View {
    let source: any MediaSource
    let playing: Bool
    let muted: Bool
    let rate: Float
    let group: String?
    let context: MediaContext
    let onEnd: () -> Void
    let content: (AudioPlayback) -> Content
    @State private var audio: AudioPlayback
    @State private var inBackground = false

    private struct Sampling: Equatable {
      let url: String
      let active: Bool
    }

    init(source: any MediaSource, playing: Bool, muted: Bool, rate: Float, group: String?, context: MediaContext, onEnd: @escaping () -> Void, content: @escaping (AudioPlayback) -> Content) {
      self.source = source
      self.playing = playing
      self.muted = muted
      self.rate = rate
      self.group = group
      self.context = context
      self.onEnd = onEnd
      self.content = content
      _audio = State(initialValue: AudioPlayback(item: source.item, playing: playing, muted: muted, rate: rate, group: group, places: context.places))
    }

    var body: some View {
      content(audio)
        .task(id: source.url) {
          for await value in context.places.stateStream(key: source.url) { if audio.follow(value) { onEnd() } }
        }
        .task(id: Sampling(url: source.url, active: !inBackground)) {
          guard !inBackground else { return }
          for await value in context.slots.progress.stream(for: source.url) { audio.refresh(progress: value) }
        }
        .onReceive(NotificationCenter.default.publisher(for: SystemSignalNames.system.background)) { _ in inBackground = true }
        .onReceive(NotificationCenter.default.publisher(for: SystemSignalNames.system.foreground)) { _ in inBackground = false }
        .onAppear { audio.mountPlace() }
        .onDisappear { audio.unmountPlace() }
        .onChange(of: playing) { _, value in audio.follow(playing: value) }
        .onChange(of: muted) { _, value in audio.follow(muted: value) }
        .onChange(of: rate) { _, value in audio.setRate(value) }
        .onChange(of: group) { _, value in audio.follow(group: value) }
    }
  }
}
