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
  @ObservationIgnored private let key: String
  @ObservationIgnored private let places: MediaPlaces
  @ObservationIgnored private let slots: LoadedSlots

  init(key: String, playing: Bool, rate: Float = 1, places: MediaPlaces, slots: LoadedSlots) {
    self.key = key
    self.rate = rate
    self.places = places
    self.slots = slots
    isPlaying = playing
  }

  public func toggle() {
    setPlaying(!isPlaying)
  }

  public func setRate(_ value: Float) {
    rate = value
    places.setRate(key: key, rate: value)
  }

  public func seek(to fraction: Double) {
    guard let duration = slots.slot(for: key)?.sample().durationMs else { return }
    places.seek(key: key, positionMs: UInt64(Double(duration) * min(max(fraction, 0), 1)))
  }

  func refresh(progress value: Double) {
    if progress != value { progress = value }
    if isPlaying && progress >= 1 { setPlaying(false) }
  }

  func follow(_ value: MediaState) {
    if state != value { state = value }
    if value == .ended && isPlaying { setPlaying(false) }
  }

  private func setPlaying(_ value: Bool) {
    isPlaying = value
    places.setPlaying(key: key, playing: value)
  }
}

public struct MediaAudio<Content: View>: View {

  @Environment(\.mediaContext) private var context
  private let source: any MediaSource
  private let playing: Bool
  private let muted: Bool
  private let rate: Float
  private let content: (AudioPlayback) -> Content

  public init(
    _ source: some MediaSource,
    playing: Bool = false,
    muted: Bool = false,
    rate: Float = 1,
    @ViewBuilder content: @escaping (AudioPlayback) -> Content
  ) {
    self.source = source
    self.playing = playing
    self.muted = muted
    self.rate = rate
    self.content = content
  }

  public var body: some View {
    if let context {
      Placed(source: source, playing: playing, muted: muted, rate: rate, context: context, content: content)
        .id(source.url)
        .id(ObjectIdentifier(context.places))
    }
  }

  private struct Placed: View {
    let source: any MediaSource
    let playing: Bool
    let muted: Bool
    let rate: Float
    let context: MediaContext
    let content: (AudioPlayback) -> Content
    @State private var audio: AudioPlayback
    @State private var owner = UUID()
    @State private var onScreen = false
    @State private var inBackground = false

    private struct Sampling: Equatable {
      let url: String
      let active: Bool
    }

    init(source: any MediaSource, playing: Bool, muted: Bool, rate: Float, context: MediaContext, content: @escaping (AudioPlayback) -> Content) {
      self.source = source
      self.playing = playing
      self.muted = muted
      self.rate = rate
      self.context = context
      self.content = content
      _audio = State(initialValue: AudioPlayback(key: source.url, playing: playing, rate: rate, places: context.places, slots: context.slots))
    }

    var body: some View {
      content(audio)
        .task(id: source.url) {
          for await value in context.places.stateStream(key: source.url) { audio.follow(value) }
        }
        .task(id: Sampling(url: source.url, active: onScreen && !inBackground)) {
          guard onScreen && !inBackground else { return }
          for await value in context.slots.progress.stream(for: source.url) { audio.refresh(progress: value) }
        }
        .background(OnScreenProbe { onScreen = $0 })
        .onReceive(NotificationCenter.default.publisher(for: SystemSignalNames.system.background)) { _ in inBackground = true }
        .onReceive(NotificationCenter.default.publisher(for: SystemSignalNames.system.foreground)) { _ in inBackground = false }
        .onAppear {
          let claims = KeyClaims.of(context.places)
          claims.visible(owner: owner, source: source.item, visible: true)
          claims.setPlaying(owner: owner, key: source.url, playing: audio.isPlaying)
          claims.setMuted(owner: owner, key: source.url, muted: muted)
          context.places.setRate(key: source.url, rate: audio.rate)
        }
        .onDisappear {
          let claims = KeyClaims.of(context.places)
          claims.release(owner: owner, key: source.url)
        }
        .onChange(of: muted) { _, value in KeyClaims.of(context.places).setMuted(owner: owner, key: source.url, muted: value) }
        .onChange(of: rate) { _, value in audio.setRate(value) }
    }
  }
}
