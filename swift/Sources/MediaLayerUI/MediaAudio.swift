import MediaLayer
import Observation
import SwiftUI

@MainActor
@Observable
public final class AudioPlayback {
  public private(set) var isPlaying: Bool
  public private(set) var progress = 0.0
  public private(set) var state: MediaState = .idle
  @ObservationIgnored private let key: String
  @ObservationIgnored private let places: MediaPlaces
  @ObservationIgnored private let slots: LoadedSlots

  init(key: String, playing: Bool, places: MediaPlaces, slots: LoadedSlots) {
    self.key = key
    self.places = places
    self.slots = slots
    isPlaying = playing
  }

  public func toggle() {
    setPlaying(!isPlaying)
  }

  public func seek(to fraction: Double) {
    guard let duration = slots.slot(for: key)?.sample().durationMs else { return }
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

public struct MediaAudio<Content: View>: View {
  private static var sampleInterval: Duration { .milliseconds(250) }

  @Environment(\.mediaContext) private var context
  private let source: any MediaSource
  private let playing: Bool
  private let muted: Bool
  private let content: (AudioPlayback) -> Content

  public init(
    _ source: some MediaSource,
    playing: Bool = false,
    muted: Bool = false,
    @ViewBuilder content: @escaping (AudioPlayback) -> Content
  ) {
    self.source = source
    self.playing = playing
    self.muted = muted
    self.content = content
  }

  public var body: some View {
    if let context {
      Placed(source: source, playing: playing, muted: muted, context: context, content: content)
    }
  }

  private struct Placed: View {
    let source: any MediaSource
    let playing: Bool
    let muted: Bool
    let context: MediaContext
    let content: (AudioPlayback) -> Content
    @State private var audio: AudioPlayback

    init(source: any MediaSource, playing: Bool, muted: Bool, context: MediaContext, content: @escaping (AudioPlayback) -> Content) {
      self.source = source
      self.playing = playing
      self.muted = muted
      self.context = context
      self.content = content
      _audio = State(initialValue: AudioPlayback(key: source.url, playing: playing, places: context.places, slots: context.slots))
    }

    var body: some View {
      content(audio)
        .task(id: source.url) {
          for await value in context.places.stateStream(key: source.url) { audio.follow(value) }
        }
        .task(id: source.url) {
          while !Task.isCancelled {
            audio.refresh(from: context.slots.slot(for: source.url)?.sample())
            try? await Task.sleep(for: MediaAudio.sampleInterval)
          }
        }
        .onAppear {
          context.places.visible(source: source.item, visible: true)
          context.places.setPlaying(key: source.url, playing: audio.isPlaying)
          context.places.setMuted(key: source.url, muted: muted)
        }
        .onDisappear { context.places.visible(source: source.item, visible: false) }
        .onChange(of: muted) { _, value in context.places.setMuted(key: source.url, muted: value) }
    }
  }
}
