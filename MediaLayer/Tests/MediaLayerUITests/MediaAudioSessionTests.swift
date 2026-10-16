import Foundation
import Testing
@testable import MediaLayerUI

struct MediaAudioSessionTests {
  private func category(_ focus: MediaAudioFocus) -> String {
    FakeAudioSessionPort.category(.playback(focus))
  }

  @Test
  func constructionAndIdleSlotsNeverTouchTheSession() async {
    let port = FakeAudioSessionPort()
    let session = MediaAudioSession(focus: .duck, port: port, releaseDelay: .zero)

    session.setAudible(slot: 1, false)
    #expect(await session.settled())

    #expect(port.log.isEmpty)
  }

  @Test
  func audibleSlotsSetTheCategoryAndActivateOnceAndTheLastOneReleasesTheFocus() async {
    let port = FakeAudioSessionPort()
    let session = MediaAudioSession(focus: .duck, port: port, releaseDelay: .zero)

    session.setAudible(slot: 1, true)
    #expect(await session.settled())
    session.setAudible(slot: 2, true)
    session.setAudible(slot: 1, false)
    #expect(await session.settled())
    #expect(port.log == [category(.duck), "active true"])

    session.setAudible(slot: 2, false)
    #expect(await session.settled())
    #expect(port.log == [category(.duck), "active true", "active false"])
    #expect(port.onMain == 0)
  }

  @Test
  func recordingUsesItsOwnPlanAndLeavesTheCategoryAloneWhenItEnds() async {
    let port = FakeAudioSessionPort()
    let session = MediaAudioSession(focus: .mix, port: port, releaseDelay: .zero)

    #expect(await session.activateRecording())
    await session.deactivateRecording()
    #expect(port.log == [FakeAudioSessionPort.category(.recording), "active true", "active false"])

    session.setAudible(slot: 1, true)
    #expect(await session.settled())
    #expect(port.log.suffix(2) == [category(.mix), "active true"])
  }

  @Test
  func playbackDuringRecordingDoesNotChangeTheMicrophoneCategory() async {
    let port = FakeAudioSessionPort()
    let session = MediaAudioSession(focus: .mix, port: port, releaseDelay: .zero)
    #expect(await session.activateRecording())

    session.setAudible(slot: 1, true)
    session.setAudible(slot: 1, false)
    #expect(await session.settled())

    #expect(port.log == [FakeAudioSessionPort.category(.recording), "active true"])
  }

  @Test
  func aPageChangeThatPausesBeforeItPlaysDoesNotReleaseTheFocus() async {
    let port = FakeAudioSessionPort()
    let session = MediaAudioSession(focus: .duck, port: port, releaseDelay: .milliseconds(150))
    session.setAudible(slot: 1, true)
    #expect(await session.settled())

    session.setAudible(slot: 1, false)
    async let released = session.settled()
    try? await Task.sleep(for: .milliseconds(30))
    session.setAudible(slot: 2, true)
    #expect(await released)

    #expect(port.log == [category(.duck), "active true"])
  }

  @Test
  func aBusyDeactivationIsRetriedUntilTheSessionLetsGo() async {
    let port = FakeAudioSessionPort(busyDeactivations: 2)
    let session = MediaAudioSession(focus: .mix, port: port, releaseDelay: .zero)
    session.setAudible(slot: 1, true)
    #expect(await session.settled())

    session.setAudible(slot: 1, false)
    #expect(await session.settled())

    #expect(port.log.suffix(3) == ["active false BUSY", "active false BUSY", "active false"])
  }

  private func readOnePress(_ session: MediaAudioSession, _ port: FakeAudioSessionPort) async -> (Float, Float?) {
    guard let reading = await session.readVolume() else { return (-1, nil) }

    port.pressVolume(0.7)
    var first: Float?
    for await volume in reading.changes {
      first = volume
      break
    }
    return (reading.initial, first)
  }

  @Test
  func readingTheVolumeHoldsANonDuckingSessionUntilTheStreamEnds() async {
    let port = FakeAudioSessionPort()
    let session = MediaAudioSession(focus: .duck, port: port, releaseDelay: .zero)

    let (initial, pressed) = await readOnePress(session, port)

    #expect(initial == 0.5)
    #expect(pressed == 0.7)
    #expect(port.log.prefix(2) == [category(.mix), "active true"])
    await eventually("session released") { port.log.last == "active false" }
  }

  @Test
  func aFailedRecordingActivationLeavesNoRecordingHeld() async {
    let port = FakeAudioSessionPort(failActivation: true)
    let session = MediaAudioSession(focus: .mix, port: port, releaseDelay: .zero)

    #expect(await session.activateRecording() == false)
    #expect(!session.isRecording)
  }

  @Test
  func isRecordingFollowsTheActivationPair() async {
    let session = MediaAudioSession(focus: .mix, port: FakeAudioSessionPort(), releaseDelay: .zero)

    #expect(await session.activateRecording())
    #expect(session.isRecording)
    await session.deactivateRecording()
    #expect(!session.isRecording)
  }
}
