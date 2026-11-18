import Foundation
import MediaLayer
import MediaLayerTesting
import Testing
@testable import MediaLayerUI

@Suite(.serialized, .timeLimit(.minutes(1)))
struct AudioDriverTests {
  @Test
  func inv_audio_80_a_call_of_another_app_ends_a_voice_take_with_in_call_and_names_it_in_availability() async throws {
    let app = try await AppLayer.open()
    let availability = Collected(app.media.microphoneAvailability)
    let recorder = MediaVoiceRecorder(capture: WritingCapture(), maxDurationMs: 60_000, link: { [media = app.media] in media.audioLink })
    try await recorder.start(to: temporaryFile("m4a"))
    let driver = MediaAudioDriver(app.media)

    driver.startCall()

    await eventually("in call") { recorder.state.outcome == .refused(reason: .inCall) }
    await eventually("external call") { availability.last == .externalCall }
    driver.endCall()
    await eventually("available again") { availability.last == .available }
    await app.close()
  }

  @Test
  func inv_audio_81_a_silenced_microphone_ends_a_voice_take_with_microphone_silent() async throws {
    let app = try await AppLayer.open()
    let recorder = MediaVoiceRecorder(capture: WritingCapture(), maxDurationMs: 60_000, link: { [media = app.media] in media.audioLink })
    try await recorder.start(to: temporaryFile("m4a"))

    MediaAudioDriver(app.media).silenceMicrophone()

    await eventually("silent") { recorder.state.outcome == .refused(reason: .microphoneSilent) }
    await app.close()
  }

  @Test
  func inv_audio_81_a_silenced_input_shows_no_sound_and_comes_back_with_the_sound() async throws {
    let app = try await AppLayer.open()
    let availability = Collected(app.media.microphoneAvailability)
    let link = try #require(app.media.audioLink)
    let owner = LiveInputOwner()
    let handle = link.openInput(.voice, owner: owner)
    await eventually("input live") { owner.generation != nil }
    let driver = MediaAudioDriver(app.media)

    driver.silenceMicrophone()
    await eventually("no sound") { availability.last == .noSound }
    driver.restoreMicrophone()

    await eventually("sound again") { availability.last == .available }
    handle.close()
    await app.close()
  }

  @Test
  func inv_audio_82_an_interruption_and_a_revoked_permission_end_a_voice_take_and_show_in_availability() async throws {
    let app = try await AppLayer.open()
    let availability = Collected(app.media.microphoneAvailability)
    let driver = MediaAudioDriver(app.media)
    let interrupted = MediaVoiceRecorder(capture: WritingCapture(), maxDurationMs: 60_000, link: { [media = app.media] in media.audioLink })
    try await interrupted.start(to: temporaryFile("m4a"))

    driver.beginInterruption()
    await eventually("interrupted") { interrupted.state.outcome == .refused(reason: .interrupted) }
    driver.endInterruption()

    let revoked = MediaVoiceRecorder(capture: WritingCapture(), maxDurationMs: 60_000, link: { [media = app.media] in media.audioLink })
    await eventually("start again") { (try? await revoked.start(to: temporaryFile("m4a"))) != nil }
    driver.revokePermission()
    await eventually("revoked") { revoked.state.outcome == .refused(reason: .permissionDenied) }
    await eventually("denied") { availability.last == .permissionDenied }
    driver.grantPermission()
    await eventually("granted") { availability.last == .available }
    await app.close()
  }
}

private final class LiveInputOwner: AudioInputOwner, @unchecked Sendable {
  private let lock = NSLock()
  private var live: UInt64?
  private var heardOnce = false
  var generation: UInt64? { lock.withLock { live } }
  /// Only the first restore hears sound: a later restore of the core must not hide the silence the test drives.
  func restore(_ input: AudioInputHandle, generation: UInt64) {
    let first = lock.withLock { () -> Bool in live = generation; defer { heardOnce = true }; return !heardOnce }
    if first { input.sound(generation: generation, heard: true) }
    TestSignal.shared.fire()
  }
  func stop(generation: UInt64) { lock.withLock { live = nil } }
}
