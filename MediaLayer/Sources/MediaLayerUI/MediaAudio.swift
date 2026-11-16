import MediaLayer
import Observation
import SwiftUI

func audioSounding(playing: Bool, state: MediaState) -> Bool {
  switch state {
  case .ready, .rendered: playing
  case .idle, .loading, .ended, .failed: false
  }
}

func audioBuffering(wish: Bool, state: MediaState) -> Bool {
  switch state {
  case .idle, .loading: wish
  case .ready, .rendered, .ended, .failed: false
  }
}

@MainActor
@Observable
public final class AudioPlayback {
  public private(set) var playback = PlaybackState(playing: false, wish: false, reason: nil, refused: nil, attempt: 0)
  public var isPlaying: Bool { playback.playing }
  public private(set) var progress = 0.0
  public private(set) var playhead = AudioPlayhead()
  @ObservationIgnored private var jumpPending = false
  public private(set) var rate: Float
  public private(set) var state: MediaState = .idle { didSet { TestChange.fire() } }
  @ObservationIgnored private let item: MediaItem
  @ObservationIgnored private(set) var group: String?
  @ObservationIgnored private var muted: Bool
  @ObservationIgnored private let places: MediaPlaces
  @ObservationIgnored private var mount: PlaceMount?
  @ObservationIgnored private var playingWish: Bool
  @ObservationIgnored private var playGeneration: UInt64 = 0
  @ObservationIgnored private let awake = ScreenAwakeHold()
  @ObservationIgnored private let gate = DeliveryGate()

  init(item: MediaItem, playing: Bool, muted: Bool = false, rate: Float = 1, group: String? = nil, places: MediaPlaces) {
    self.item = item
    self.group = group
    self.muted = muted
    self.rate = rate
    self.places = places
    playingWish = playing
  }

  func mountPlace() {
    guard mount == nil else { return }
    let token = gate.open()
    let relay = PlaybackRelay(receive: { [weak self, gate] value in
      gate.deliver(token) {
        self?.playback = value
        self?.awake.set(playing: value.playing)
      }
    }, failure: { [weak self, gate] error in
      gate.deliver(token) { self?.follow(.failed(detail: String(describing: error))) }
    })
    mount = PlaceMount(places: places, item: item, playback: relay) { [weak self, gate] error in
      gate.deliver(token) { self?.follow(.failed(detail: String(describing: error))) }
    }
    push()
  }

  func unmountPlace() {
    gate.close()
    awake.set(playing: false)
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
    mount?.update(.make(visible: true, playing: playingWish, group: group, muted: muted, rate: rate, playGeneration: playGeneration))
  }

  public func toggle() {
    setPlaying(!isPlaying)
  }

  public func play() { setPlaying(true) }

  public func pause() { setPlaying(false) }

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
    jumpPending = true
    playhead = AudioPlayhead(anchor: PlayheadAnchor(fraction: clamped, atMs: AudioPlayhead.milliseconds(Date()), perMs: playhead.anchor.perMs))
    mount?.seek(clamped)
  }

  func refresh(report: PlayheadReport) {
    var report = report
    if !report.settling && jumpPending {
      report.jump = true
      jumpPending = false
    }
    MediaTrace.emit("playhead", "report pos=\(report.positionMs) dur=\(report.durationMs) playing=\(report.playing) rate=\(report.rate) settling=\(report.settling)")
    if !report.settling && report.durationMs > 0 {
      refresh(progress: min(max(Double(report.positionMs) / Double(report.durationMs), 0), 1))
    }
    playhead = AudioPlayhead(anchor: playheadNext(shown: playhead.anchor, report: report))
  }

  func refresh(progress value: Double) {
    if progress != value { progress = value }
  }

  public var isSounding: Bool {
    audioSounding(playing: isPlaying, state: state)
  }

  public var isBuffering: Bool {
    audioBuffering(wish: playback.wish, state: state)
  }

  @discardableResult
  func follow(_ value: MediaState) -> Bool {
    let isNewEnd = value == .ended && state != .ended
    if state != value { state = value }
    guard value == .ended else { return false }
    if isPlaying { setPlaying(false) }
    guard isNewEnd else { return false }
    refresh(progress: 0)
    jumpPending = true
    playhead = AudioPlayhead()
    mount?.seek(0)
    return true
  }

  private func setPlaying(_ value: Bool) {
    playingWish = value
    if value { playGeneration &+= 1 }
    push()
  }
}

public struct AudioPlayhead: Sendable {
  public let anchor: PlayheadAnchor

  public init(anchor: PlayheadAnchor = PlayheadAnchor(fraction: 0, atMs: 0, perMs: 0)) {
    self.anchor = anchor
  }

  public func progress(at date: Date = Date()) -> Double {
    // same arithmetic as playhead_at in the core
    let now = Self.milliseconds(date)
    let elapsed = now > anchor.atMs ? Double(now - anchor.atMs) : 0
    return min(max(anchor.fraction + elapsed * anchor.perMs, 0), 1)
  }

  static func milliseconds(_ date: Date) -> UInt64 {
    UInt64(max(0, date.timeIntervalSince1970) * 1000)
  }
}

public struct MediaAudioProgress<Content: View>: View {
  private let audio: AudioPlayback
  private let content: (Double) -> Content

  public init(_ audio: AudioPlayback, @ViewBuilder content: @escaping (Double) -> Content) {
    self.audio = audio
    self.content = content
  }

  public var body: some View {
    TimelineView(.animation(paused: !audio.isSounding)) { context in
      content(audio.playhead.progress(at: context.date))
    }
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
          let key = await SourceKey.of(source.item)
          guard !Task.isCancelled else { return }
          for await value in context.slots.progress.stream(for: key) { audio.refresh(progress: value) }
        }
        .task(id: Sampling(url: source.url, active: !inBackground)) {
          guard !inBackground else { return }
          let key = await SourceKey.of(source.item)
          guard !Task.isCancelled else { return }
          for await report in context.slots.progress.reports(for: key) { audio.refresh(report: report) }
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
