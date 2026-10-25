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
  @ObservationIgnored let owner = UUID()
  @ObservationIgnored private let key: String
  @ObservationIgnored private(set) var group: String?
  @ObservationIgnored private let places: MediaPlaces

  init(key: String, playing: Bool, rate: Float = 1, group: String? = nil, places: MediaPlaces) {
    self.key = key
    self.group = group
    self.rate = rate
    self.places = places
    isPlaying = playing
  }

  public func toggle() {
    setPlaying(!isPlaying)
  }

  func follow(playing value: Bool) {
    setPlaying(value)
  }

  func follow(group value: String?) {
    group = value
  }

  public func setRate(_ value: Float) {
    rate = value
    places.setRate(key: key, rate: value)
  }

  public func seek(to fraction: Double) {
    guard fraction.isFinite else { return }
    let clamped = min(max(fraction, 0), 1)
    refresh(progress: clamped)
    places.seek(key: key, fraction: clamped)
  }

  func refresh(progress value: Double) {
    if progress != value { progress = value }
  }

  func follow(_ value: MediaState) {
    if state != value { state = value }
    if value == .ended && isPlaying { setPlaying(false) }
  }

  private func setPlaying(_ value: Bool) {
    isPlaying = value
    KeyClaims.of(places).setPlaying(owner: owner, key: key, playing: value, group: group)
  }
}

public struct MediaAudio<Content: View>: View {

  @Environment(\.mediaContext) private var context
  private let source: any MediaSource
  private let playing: Bool
  private let muted: Bool
  private let rate: Float
  private let group: String?
  private let content: (AudioPlayback) -> Content

  public init(
    _ source: some MediaSource,
    playing: Bool = false,
    muted: Bool = false,
    rate: Float = 1,
    group: String? = nil,
    @ViewBuilder content: @escaping (AudioPlayback) -> Content
  ) {
    self.source = source
    self.playing = playing
    self.muted = muted
    self.rate = rate
    self.group = group
    self.content = content
  }

  public var body: some View {
    if let context {
      Placed(source: source, playing: playing, muted: muted, rate: rate, group: group, context: context, content: content)
        .id(source.url)
        .id(ObjectIdentifier(context.places))
    }
  }

  private struct Placed: View {
    let source: any MediaSource
    let playing: Bool
    let muted: Bool
    let rate: Float
    let group: String?
    let context: MediaContext
    let content: (AudioPlayback) -> Content
    @State private var audio: AudioPlayback
    @State private var inBackground = false

    private struct Sampling: Equatable {
      let url: String
      let active: Bool
    }

    init(source: any MediaSource, playing: Bool, muted: Bool, rate: Float, group: String?, context: MediaContext, content: @escaping (AudioPlayback) -> Content) {
      self.source = source
      self.playing = playing
      self.muted = muted
      self.rate = rate
      self.group = group
      self.context = context
      self.content = content
      _audio = State(initialValue: AudioPlayback(key: source.url, playing: playing, rate: rate, group: group, places: context.places))
    }

    var body: some View {
      content(audio)
        .task(id: source.url) {
          for await value in context.places.stateStream(key: source.url) { audio.follow(value) }
        }
        .task(id: Sampling(url: source.url, active: !inBackground)) {
          guard !inBackground else { return }
          for await value in context.slots.progress.stream(for: source.url) { audio.refresh(progress: value) }
        }
        .onReceive(NotificationCenter.default.publisher(for: SystemSignalNames.system.background)) { _ in inBackground = true }
        .onReceive(NotificationCenter.default.publisher(for: SystemSignalNames.system.foreground)) { _ in inBackground = false }
        .onAppear {
          let claims = KeyClaims.of(context.places)
          claims.visible(owner: audio.owner, source: source.item, visible: true)
          claims.setPlaying(owner: audio.owner, key: source.url, playing: audio.isPlaying, group: group)
          claims.setMuted(owner: audio.owner, key: source.url, muted: muted)
          context.places.setRate(key: source.url, rate: audio.rate)
        }
        .onDisappear {
          let claims = KeyClaims.of(context.places)
          claims.release(owner: audio.owner, key: source.url)
        }
        .onChange(of: playing) { _, value in audio.follow(playing: value) }
        .onChange(of: muted) { _, value in KeyClaims.of(context.places).setMuted(owner: audio.owner, key: source.url, muted: value) }
        .onChange(of: rate) { _, value in audio.setRate(value) }
        .onChange(of: group) { _, value in
          audio.follow(group: value)
          KeyClaims.of(context.places).setPlaying(owner: audio.owner, key: source.url, playing: audio.isPlaying, group: value)
        }
    }
  }
}
