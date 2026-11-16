@preconcurrency import AVFoundation
import Foundation
import MediaLayer
import Testing
@testable import MediaLayerUI

private func refusedApply(_ code: Int, domain: String = NSOSStatusErrorDomain) -> NSError {
  NSError(domain: domain, code: code, userInfo: [NSLocalizedDescriptionKey: "refused \(code)"])
}

private final class MuteProbe: PlayerSlotFactory, @unchecked Sendable {
  private let lock = NSLock()
  private var commands: [String] = []
  var log: [String] { lock.withLock { commands } }

  func create(slot: UInt32) async -> CreatedPlayer { CreatedPlayer(player: Slot(owner: self)) }
  fileprivate func note(_ command: String) { lock.withLock { commands.append(command) }; TestSignal.shared.fire() }

  private final class Slot: PlayerSlot, @unchecked Sendable {
    private weak var owner: MuteProbe?
    init(owner: MuteProbe) { self.owner = owner }
    func apply(command: SlotCall) async {
      switch command {
      case .load: owner?.note("load")
      case .mute: owner?.note("mute")
      case .unmute: owner?.note("unmute")
      case .play: owner?.note("play")
      default: break
      }
    }
    func dispose() async {}
    func sample() -> SlotSample { SlotSample(positionMs: 0, durationMs: nil, isLooping: false, viewAttached: false, frameRendered: false) }
  }
}

extension GlobalMedia {
  @Suite(.serialized, .timeLimit(.minutes(2)))
  struct AppSignalsTests {
    // K1
    @Test
    func inv_call_14_the_system_activation_and_deactivation_of_a_callkit_call_reach_its_states() async throws {
      let app = try await AppLayer.open()
      let call = try await app.media.callAudio(.callKit(UUID()))
      let states = Collected(call.states)

      call.systemActivated()
      await eventually("live") { states.last == .live }
      call.systemDeactivated()

      await eventually("interrupted") { states.last == .interrupted }
      #expect(states.values == [.connecting, .live, .interrupted])
      call.end()
      await app.close()
    }

    // K2
    @Test(arguments: ["interruption", "reset", "external call", "failed apply"])
    func inv_call_15_a_lost_call_sound_shows_interrupted_before_any_new_live(cause: String) async throws {
      let app = try await AppLayer.open()
      if cause == "failed apply" { app.session.failApply(.playAndRecord, refusedApply(9)) }
      let call = try await app.media.callAudio(.unmanaged)
      let states = Collected(call.states)

      switch cause {
      case "interruption":
        await eventually("live") { states.last == .live }
        app.session.send(.interruptionBegan)
      case "reset":
        await eventually("live") { states.last == .live }
        app.session.send(.mediaServicesReset)
      case "external call":
        await eventually("live") { states.last == .live }
        app.calls.send(["cellular"])
      default: break
      }

      let expected: [CallState] = cause == "failed apply" ? [.connecting, .interrupted] : [.connecting, .live, .interrupted]
      await eventually("\(cause): interrupted") { states.values.count >= expected.count }
      #expect(Array(states.values.prefix(expected.count)) == expected, "\(cause): the loss shows interrupted before any new live")
      call.end()
      await app.close()
    }

    // K3
    @Test(arguments: ["unmanaged on", "unmanaged off", "unmanaged failure", "callkit on"])
    func inv_call_16_a_speaker_choice_reaches_the_session_and_the_applied_route(choice: String) async throws {
      let app = try await AppLayer.open()
      let call = try await app.media.callAudio(choice == "callkit on" ? .callKit(UUID()) : .unmanaged)
      let route = Collected(call.route)
      await eventually("the first route") { !route.values.isEmpty }
      #expect(route.values.first == .receiver, "a new call starts on the receiver")
      let expected: CallRoute
      switch choice {
      case "unmanaged on":
        call.setSpeaker(true)
        expected = .speaker
      case "unmanaged off":
        call.setSpeaker(true)
        await eventually("speaker") { route.last == .speaker }
        call.setSpeaker(false)
        expected = .receiver
      case "unmanaged failure":
        app.session.failSpeaker(refusedApply(7, domain: "speaker"))
        call.setSpeaker(true)
        expected = .failed
      default:
        call.systemActivated()
        call.setSpeaker(true)
        expected = .speaker
      }

      await eventually("\(choice): applied route") { route.last == expected }
      let override: String = switch expected {
      case .speaker: "speaker true"
      case .receiver: "speaker false"
      default: "speaker failed"
      }
      #expect(app.session.log.last == override, "\(choice): \(app.session.log)")
      call.end()
      await app.close()
    }

    // K4: the end by the layer close is in inv_ios_04
    @Test
    func inv_call_17_an_ended_call_shows_ended_and_its_streams_finish() async throws {
      let app = try await AppLayer.open()
      let call = try await app.media.callAudio(.unmanaged)
      let states = Collected(call.states)
      let route = Collected(call.route)

      call.end()

      await eventually("ended") { states.finished && route.finished }
      #expect(states.last == .ended)
      await app.close()
    }

    // M1
    @Test
    func inv_audio_60_a_permission_change_in_the_foreground_reaches_the_availability() async throws {
      let microphone = SwitchMicrophone(granted: false)
      let app = try await AppLayer.open(microphone: microphone)
      let availability = Collected(app.media.microphoneAvailability)
      await eventually("denied at open") { availability.last == .permissionDenied }

      microphone.set(true)
      await eventually("granted") { availability.last == .available }
      microphone.set(false)

      await eventually("revoked") { availability.last == .permissionDenied }
      #expect(availability.values == [.permissionDenied, .available, .permissionDenied])
      await app.close()
    }

    // M2
    @Test
    func inv_audio_61_a_microphone_held_by_another_app_shows_other_app() async throws {
      let app = try await AppLayer.open()
      let availability = Collected(app.media.microphoneAvailability)
      app.session.failApply(.playAndRecord, refusedApply(AVAudioSession.ErrorCode.insufficientPriority.rawValue))
      let recorder = MediaVoiceRecorder(capture: WritingCapture(), maxDurationMs: 60_000)

      await #expect(throws: MediaError.self) { try await recorder.start(to: temporaryFile("m4a")) }

      await eventually("other app") { availability.last == .otherApp }
      #expect(availability.values == [.available, .otherApp])
      await app.close()
    }

    // M3
    @Test(arguments: ["interruption", "refused session"])
    func inv_audio_62_a_lost_or_refused_session_is_not_available_and_comes_back(cause: String) async throws {
      let app = try await AppLayer.open()
      let availability = Collected(app.media.microphoneAvailability)
      await eventually("available") { availability.last == .available }

      if cause == "interruption" {
        app.session.send(.interruptionBegan)
        await eventually("lost") { availability.last != .available }
        app.session.send(.interruptionEnded(shouldResume: false))
      } else {
        app.session.failApply(.playAndRecord, refusedApply(1))
        let refused = MediaVoiceRecorder(capture: WritingCapture(), maxDurationMs: 60_000)
        await #expect(throws: MediaError.self) { try await refused.start(to: temporaryFile("m4a")) }
        await eventually("refused") { availability.last != .available }
        app.session.failApply(.playAndRecord, nil)
        let recorder = MediaVoiceRecorder(capture: WritingCapture(), maxDurationMs: 60_000)
        try await recorder.start(to: temporaryFile("m4a"))
        _ = try? await recorder.stop()
      }

      await eventually("\(cause): available again") { availability.last == .available }
      #expect(availability.values.count == 3, "\(cause): \(availability.values)")
      await app.close()
    }

    // F1
    @Test
    func inv_audio_63_two_native_errors_are_two_ordered_failure_events() async throws {
      let app = try await AppLayer.open()
      let failures = Collected(Media.failures)
      let call = try await app.media.callAudio(.unmanaged)
      let states = Collected(call.states)
      await eventually("live") { states.last == .live }
      app.session.failSpeaker(refusedApply(7, domain: "speaker"))
      call.setSpeaker(true)
      call.end()
      app.session.failApply(.playAndRecord, refusedApply(9, domain: "session"))
      let recorder = MediaVoiceRecorder(capture: WritingCapture(), maxDurationMs: 60_000)
      await #expect(throws: MediaError.self) { try await recorder.start(to: temporaryFile("m4a")) }
      try Media.setMuted(false)

      await eventually("both failures") { failures.values.count >= 2 }
      #expect(failures.values.prefix(2).map { "\($0.source) \($0.domain) \($0.code)" } == ["speaker speaker 7", "session session 9"])
      await app.close()
    }

    // F2
    @Test
    func inv_audio_64_a_down_and_up_burst_of_the_volume_is_one_rise() async throws {
      let app = try await AppLayer.open()
      let raises = Collected(try #require(await Media.audio?.volumeRaises()))
      await eventually("observing the volume") { app.session.volumeObserved }

      app.session.pressVolume(0.3)
      app.session.pressVolume(0.5)
      await eventually("one rise") { raises.values.count == 1 }
      app.session.pressVolume(0.6)

      await eventually("the next rise") { raises.values.count == 2 }
      #expect(raises.values.count == 2, "the burst gave more than one rise")
      await app.close()
    }

    // F3
    @Test
    func inv_audio_65_the_global_mute_survives_a_reopen_and_a_refusal_keeps_the_flag() async throws {
      let first = try await AppLayer.open()
      try Media.setMuted(true)
      await first.close()
      let slots = MuteProbe()
      let app = try await AppLayer.open(slots: slots)
      #expect(Media.muted)
      let player = try await app.media.playUrl(source: MediaItem(kind: .video, url: appFixture("silent", "mp4"), headers: [:], cover: nil, blurHash: nil, strategy: .window))

      try player.play()

      await eventually("the native player plays") { slots.log.contains("play") }
      #expect(slots.log.contains("mute") && !slots.log.contains("unmute"), "\(slots.log)")
      player.release()
      try Media.setMuted(false)
      await app.close()

      let refusing = RecordingPlaces()
      refusing.mutedAllError = .Overloaded(detail: "full")
      Media.current = refusing
      #expect(throws: MediaError.self) { try Media.setMuted(true) }
      #expect(Media.muted == false)
      #expect(Media.failure == .Overloaded(detail: "full"))
      Media.current = nil
    }
  }
}
