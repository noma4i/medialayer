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
  public private(set) var pollEpoch = 0
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
    pollEpoch += 1
    places.seek(key: key, positionMs: UInt64(Double(duration) * min(max(fraction, 0), 1)))
  }

  func refresh(from sample: SlotSample?) {
    guard let sample, let duration = sample.durationMs, duration > 0 else { return }
    progress = min(Double(sample.positionMs) / Double(duration), 1)
    if isPlaying && progress >= 1 { setPlaying(false) }
  }

  func follow(_ value: MediaState) {
    state = value
  }

  private func setPlaying(_ value: Bool) {
    isPlaying = value
    places.setPlaying(key: key, playing: value)
  }
}

enum AudioSampling {
  static let interval: Duration = .milliseconds(250)
  static let pausedSamples = 4

  static func remaining(isPlaying: Bool, pausedLeft: Int) -> Int? {
    if isPlaying { return pausedLeft }
    return pausedLeft > 0 ? pausedLeft - 1 : nil
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

    private struct Sampling: Equatable {
      let url: String
      let isPlaying: Bool
      let epoch: Int
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
        .task(id: Sampling(url: source.url, isPlaying: audio.isPlaying, epoch: audio.pollEpoch)) {
          var pausedLeft = AudioSampling.pausedSamples
          while !Task.isCancelled {
            audio.refresh(from: context.slots.slot(for: source.url)?.sample())
            guard let left = AudioSampling.remaining(isPlaying: audio.isPlaying, pausedLeft: pausedLeft) else { return }

            pausedLeft = left
            try? await Task.sleep(for: AudioSampling.interval)
          }
        }
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
