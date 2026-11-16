#if canImport(UIKit)
import AVFoundation
import Foundation
import MediaLayer
import Observation
import SwiftUI
import Testing
import UIKit
@testable import MediaLayerUI

private struct Clip: MediaSource {
  let url: String
  let kind: Kind
  var headers: [String: String] { [:] }
}

private let audioClip = appFixture("tone", "m4a")
private let videoClip = appFixture("silent", "mp4")
private let playerClip = appFixture("rotated90", "mp4")
private let barrierClip = appFixture("rotated-recording", "mp4")

private func item(_ url: String) -> MediaItem {
  MediaItem(kind: .video, url: url, headers: [:], cover: nil, blurHash: nil, strategy: .window)
}

@MainActor
@Observable
private final class Places {
  var audio: AudioPlayback?
  var videoPlaying = false
  var videoStates: [PlaybackState] = []
  var videoPlays: [Bool] { videoStates.map(\.playing).reduce(into: []) { if $0.last != $1 { $0.append($1) } } }
}

private struct PlacesView: View {
  let places: Places
  let context: MediaContext

  var body: some View {
    VStack(spacing: 0) {
      MediaAudio(Clip(url: audioClip, kind: .audio)) { playback in
        Color.clear.onAppear { places.audio = playback }
      }
      .frame(width: 100, height: 20)
      MediaVideo(Clip(url: videoClip, kind: .video), playing: places.videoPlaying, onPlaybackChange: { places.videoStates.append($0) })
        .frame(width: 100, height: 100)
    }
    .environment(\.mediaContext, context)
  }
}

/// The real audio and video wrappers and a `playUrl` player of one layer opened the way the app opens it.
@MainActor
private final class Screen {
  let app: AppLayer
  let places = Places()
  let player: MediaPlayer
  let states: Collected<PlaybackState>
  private let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 100, height: 120))
  private var watch: Watched?

  private init(app: AppLayer, player: MediaPlayer) {
    self.app = app
    self.player = player
    states = Collected(player.states)
  }

  static func open(slots: (any PlayerSlotFactory)? = nil, mounts: Bool = true) async throws -> Screen {
    let app = try await AppLayer.open(slots: slots)
    let screen = Screen(app: app, player: try await app.media.playUrl(source: item(playerClip)))
    if mounts { screen.mount() }
    return screen
  }

  private func mount() {
    let places = places
    watch = Watched { _ = places.audio?.playback; _ = places.audio?.isSounding; _ = places.videoStates }
    window.rootViewController = UIHostingController(rootView: PlacesView(places: places, context: MediaContext(places: app.media)))
    window.isHidden = false
  }

  var audio: AudioPlayback? { places.audio }

  /// The user presses Play on the player and it plays; one key plays at a time, so the other places stay paused.
  func playPlayer() async throws {
    try player.play()
    await eventually("the player plays") { player.state.playing }
  }

  func close() async {
    watch?.stop()
    window.isHidden = true
    window.rootViewController = nil
    player.release()
    await app.close()
  }
}

enum PauseSource: CaseIterable, CustomStringConvertible {
  case interruption, call, recording, outputLoss, suspension, reset

  var description: String {
    switch self {
    case .interruption: "interruption"
    case .call: "call"
    case .recording: "recording"
    case .outputLoss: "output loss"
    case .suspension: "suspension"
    case .reset: "reset"
    }
  }
}

/// Makes the system or the app take the sound away. The returned object keeps the cause alive.
@MainActor
private func takeSound(_ source: PauseSource, _ app: AppLayer) async throws -> Any? {
  switch source {
  case .interruption:
    app.session.send(.interruptionBegan)
    return nil
  case .call:
    return try await app.media.callAudio(.unmanaged)
  case .recording:
    let recorder = MediaVoiceRecorder(capture: WritingCapture(), maxDurationMs: 60_000)
    try await recorder.start(to: temporaryFile("m4a"))
    return recorder
  case .outputLoss:
    app.session.send(.routeChanged(.oldDeviceUnavailable, hadPrivateOutput: true))
    return nil
  case .suspension:
    return try app.media.suspendPlayback(allowGroups: [])
  case .reset:
    app.session.send(.mediaServicesReset)
    return nil
  }
}

private final class ProbeSlots: PlayerSlotFactory, @unchecked Sendable {
  private let lock = NSLock()
  private var made = 0
  private var disposed = 0
  private var plays = 0
  var created: Int { lock.withLock { made } }
  var disposals: Int { lock.withLock { disposed } }
  var playCommands: Int { lock.withLock { plays } }

  func create(slot: UInt32) async -> CreatedPlayer {
    lock.withLock { made += 1 }
    TestSignal.shared.fire()
    return CreatedPlayer(player: Probe(owner: self))
  }

  fileprivate func played() { lock.withLock { plays += 1 }; TestSignal.shared.fire() }
  fileprivate func gone() { lock.withLock { disposed += 1 }; TestSignal.shared.fire() }

  private final class Probe: PlayerSlot, @unchecked Sendable {
    private weak var owner: ProbeSlots?
    init(owner: ProbeSlots) { self.owner = owner }
    func apply(command: SlotCall) async {
      if case .play = command { owner?.played() }
    }
    func dispose() async { owner?.gone() }
    func sample() -> SlotSample { SlotSample(positionMs: 0, durationMs: nil, isLooping: false, viewAttached: false, frameRendered: false) }
  }
}

extension GlobalMedia {
  @MainActor
  @Suite(.serialized, .timeLimit(.minutes(3)))
  struct PlaybackTruthTests {
    // P1
    @Test
    func inv_api_49_a_user_play_and_pause_show_the_core_truth_on_audio_video_and_player() async throws {
      let screen = try await Screen.open()
      #expect(screen.player.state == PlaybackState(playing: false, wish: false, reason: nil, refused: nil, attempt: 0))
      await eventually("the audio place is mounted") { screen.audio != nil }

      screen.audio?.play()
      await eventually("the audio plays") { screen.audio?.isPlaying == true }
      screen.audio?.pause()
      await eventually("the audio shows paused") { screen.audio?.isPlaying == false && screen.audio?.isSounding == false }

      screen.places.videoPlaying = true
      await eventually("the video plays") { screen.places.videoPlays.last == true }
      screen.places.videoPlaying = false
      await eventually("the video shows paused") { screen.places.videoPlays.last == false }

      try await screen.playPlayer()
      #expect(screen.player.state == PlaybackState(playing: true, wish: true, reason: nil, refused: nil, attempt: 0))
      try screen.player.pause()
      await eventually("the player shows paused") { !screen.player.state.playing }
      #expect(screen.player.state == PlaybackState(playing: false, wish: false, reason: nil, refused: nil, attempt: 0))
      #expect(Array(screen.places.videoPlays.drop(while: { !$0 })) == [true, false], "the video showed play then pause: \(screen.places.videoStates)")
      #expect(screen.places.videoStates.last == PlaybackState(playing: false, wish: false, reason: nil, refused: nil, attempt: 0))
      await screen.close()
    }

    // P2
    @Test(arguments: [
      (PauseSource.interruption, true, PauseReason.interrupted),
      (.call, false, .inCall),
      (.recording, true, .capture),
      (.outputLoss, false, .outputLost),
      (.suspension, true, .suspended),
      (.reset, false, .mediaReset),
    ])
    func inv_api_50_every_pause_by_the_library_shows_paused_with_its_reason(source: PauseSource, keepsWish: Bool, reason: PauseReason) async throws {
      let screen = try await Screen.open(mounts: false)
      try await screen.playPlayer()

      let cause = try await takeSound(source, screen.app)

      await eventually("\(source): the player shows paused") { !screen.player.state.playing }
      #expect(screen.player.state == PlaybackState(playing: false, wish: keepsWish, reason: reason, refused: nil, attempt: 0), "\(source)")
      withExtendedLifetime(cause) {}
      await screen.close()
    }

    // P3
    @Test(arguments: [PauseSource.interruption, .suspension])
    func inv_api_51_a_resume_with_permission_shows_the_same_places_playing(source: PauseSource) async throws {
      let screen = try await Screen.open(mounts: false)
      try await screen.playPlayer()
      var cause = try await takeSound(source, screen.app)
      await eventually("\(source): paused") { !screen.player.state.playing }

      switch source {
      case .interruption: screen.app.session.send(.interruptionEnded(shouldResume: true))
      default: (cause as? PlaybackSuspension)?.release(); cause = nil
      }

      await eventually("\(source): the player plays again without a new Play") { screen.player.state.playing }
      #expect(screen.player.state == PlaybackState(playing: true, wish: true, reason: nil, refused: nil, attempt: 0))
      withExtendedLifetime(cause) {}
      await screen.close()
    }

    // P4
    @Test(arguments: ["interruption without resume", "output loss", "call"])
    func inv_api_52_a_forgotten_wish_shows_paused_and_one_play_plays_again(cause: String) async throws {
      let screen = try await Screen.open(mounts: false)
      try await screen.playPlayer()
      switch cause {
      case "interruption without resume":
        screen.app.session.send(.interruptionBegan)
        screen.app.session.send(.interruptionEnded(shouldResume: false))
      case "output loss":
        screen.app.session.send(.routeChanged(.oldDeviceUnavailable, hadPrivateOutput: true))
      default:
        let call = try await screen.app.media.callAudio(.unmanaged)
        await eventually("in call") { screen.player.state.reason == .inCall }
        call.end()
      }
      await eventually("\(cause): the wish is forgotten") { !screen.player.state.wish && !screen.player.state.playing }

      try screen.player.play()

      await eventually("\(cause): one Play plays") { screen.player.state.playing }
      #expect(screen.player.state == PlaybackState(playing: true, wish: true, reason: nil, refused: nil, attempt: 0))
      await screen.close()
    }

    // P5
    @Test(arguments: [(PauseSource.call, PlayRefusal.inCall)])
    func inv_api_53_a_play_during_a_call_is_refused(source: PauseSource, refusal: PlayRefusal) async throws {
      let screen = try await Screen.open()
      await eventually("the audio place is mounted") { screen.audio != nil }
      let cause = try await takeSound(source, screen.app)

      screen.audio?.play()
      try screen.player.play()

      await eventually("\(source): the refusal reached the player") { screen.player.state.refused == refusal }
      #expect(screen.player.state == PlaybackState(playing: false, wish: false, reason: nil, refused: refusal, attempt: 0))
      #expect(screen.audio?.isPlaying == false && screen.audio?.isSounding == false)
      withExtendedLifetime(cause) {}
      await screen.close()
    }

    // P6
    @Test
    func inv_api_54_a_mute_change_after_a_forgotten_wish_plays_nothing() async throws {
      let screen = try await Screen.open(mounts: false)
      try screen.player.play()
      await eventually("playing") { screen.player.state.playing }
      screen.app.session.send(.routeChanged(.oldDeviceUnavailable, hadPrivateOutput: true))
      await eventually("forgotten") { !screen.player.state.wish }
      let forgotten = screen.states.values.count

      try Media.setMuted(true)
      try Media.setMuted(false)
      let barrier = try await screen.app.media.playUrl(source: item(barrierClip))
      let barrierStates = Collected(barrier.states)
      try barrier.play()
      await eventually("the barrier player plays: the mute changes were applied before it") { barrierStates.last?.playing == true }

      #expect(screen.states.values.count == forgotten, "a mute change made a new state: \(screen.states.values)")
      #expect(screen.player.state == PlaybackState(playing: false, wish: false, reason: .outputLost, refused: nil, attempt: 0))
      barrier.release()
      await screen.close()
    }

    // P7
    @Test
    func inv_api_55_a_play_url_player_is_paused_by_a_call_and_one_play_after_it_plays() async throws {
      let screen = try await Screen.open(mounts: false)
      try screen.player.play()
      await eventually("playing") { screen.player.state.playing }

      let call = try await screen.app.media.callAudio(.unmanaged)
      await eventually("paused by the call") { !screen.player.state.playing }
      #expect(screen.player.state == PlaybackState(playing: false, wish: false, reason: .inCall, refused: nil, attempt: 0))
      call.end()
      try screen.player.play()

      await eventually("one Play after the call plays") { screen.player.state.playing }
      let shown = screen.states.values.map(\.playing).reduce(into: [Bool]()) { if $0.last != $1 { $0.append($1) } }
      #expect(shown == [false, true, false, true])
      await screen.close()
    }

    // P8
    @Test
    func inv_api_56_a_media_reset_disposes_the_native_players_and_a_new_play_builds_a_new_one() async throws {
      let slots = ProbeSlots()
      let screen = try await Screen.open(slots: slots, mounts: false)
      try screen.player.play()
      await eventually("a native player plays") { screen.player.state.playing && slots.playCommands == 1 }
      let built = slots.created

      screen.app.session.send(.mediaServicesReset)

      await eventually("every native player is disposed") { slots.disposals == built }
      await eventually("the reset shows on the player") { screen.player.state == PlaybackState(playing: false, wish: false, reason: .mediaReset, refused: nil, attempt: 0) }
      try screen.player.play()
      await eventually("a new native player plays") { screen.player.state.playing && slots.playCommands == 2 }
      #expect(slots.created > built)
      await screen.close()
    }

    // A05
    @Test
    func inv_api_57_every_states_stream_of_a_player_ends_after_its_release_and_after_the_close_and_so_does_a_later_one() async throws {
      let app = try await AppLayer.open()
      let released = try await app.media.playUrl(source: item(playerClip))
      let early = Collected(released.states)
      await eventually("the released player showed its state") { early.last != nil }

      released.release()

      await eventually("the release ended the stream") { early.finished }
      let late = Collected(released.states)
      await eventually("a stream asked after the release ended") { late.finished }
      #expect(late.values.isEmpty, "a stream asked after the release showed \(late.values)")

      let closed = try await app.media.playUrl(source: item(barrierClip))
      let beforeClose = Collected(closed.states)
      await eventually("the second player showed its state") { beforeClose.last != nil }
      await app.close()
      await eventually("the close ended the stream") { beforeClose.finished }
      let afterClose = Collected(closed.states)
      await eventually("a stream asked after the close ended") { afterClose.finished }
      #expect(afterClose.values.isEmpty, "a stream asked after the close showed \(afterClose.values)")
    }

    // A28
    @Test
    func inv_api_58_the_audio_and_video_places_show_the_whole_playback_state_with_the_pause_reason_and_the_refusal() async throws {
      let screen = try await Screen.open()
      await eventually("the audio place is mounted") { screen.audio != nil }

      screen.places.videoPlaying = true
      await eventually("the video plays") { screen.places.videoStates.last?.playing == true }
      screen.app.session.send(.interruptionBegan)
      await eventually("the interruption paused the video") { screen.places.videoStates.last?.playing == false }
      #expect(screen.places.videoStates.last == PlaybackState(playing: false, wish: true, reason: .interrupted, refused: nil, attempt: 0))
      screen.app.session.send(.interruptionEnded(shouldResume: true))
      await eventually("the video plays again") { screen.places.videoStates.last?.playing == true }
      screen.places.videoPlaying = false
      await eventually("the video shows paused") { screen.places.videoStates.last == PlaybackState(playing: false, wish: false, reason: nil, refused: nil, attempt: 0) }

      screen.audio?.play()
      await eventually("the audio plays") { screen.audio?.playback.playing == true }
      let call = try await screen.app.media.callAudio(.unmanaged)
      await eventually("the call paused the audio") { screen.audio?.playback.playing == false }
      #expect(screen.audio?.playback == PlaybackState(playing: false, wish: false, reason: .inCall, refused: nil, attempt: 0))

      screen.audio?.play()
      screen.places.videoPlaying = true

      await eventually("the audio play in the call is refused") { screen.audio?.playback.refused == .inCall }
      #expect(screen.audio?.playback == PlaybackState(playing: false, wish: false, reason: nil, refused: .inCall, attempt: 0))
      await eventually("the video play in the call is refused") { screen.places.videoStates.last?.refused == .inCall }
      #expect(screen.places.videoStates.last == PlaybackState(playing: false, wish: false, reason: nil, refused: .inCall, attempt: 0))
      call.end()
      await screen.close()
    }
  }
}
#endif
