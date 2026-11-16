import AVFoundation
import Foundation
import MediaLayer
import Testing
@testable import MediaLayerUI

extension GlobalMedia {
  @Suite(.serialized, .timeLimit(.minutes(1)))
  struct MediaAudioSlotTests {
    private static var clip: String {
      Bundle.module.url(forResource: "silent", withExtension: "mp4", subdirectory: "Fixtures")!.absoluteString
    }

    private func session(_ focus: MediaAudioFocus, _ port: FakeAudioSessionPort) -> MediaAudioSession {
      MediaAudioSession(focus: focus, port: port, releaseDelay: .milliseconds(150))
    }

    private func makeSlot(startMuted: Bool, session: MediaAudioSession) async -> AVQueuePlayerSlot {
      let factory = AVQueuePlayerSlotFactory(startMuted: startMuted, audio: session, slots: LoadedSlots())
      factory.events.bind(PlayerEventLog())
      return await factory.makeSlot(7)
    }

    private func drain(_ audio: MediaAudioSession) async {
      await MainActor.run {}
      _ = await audio.settled()
      await MainActor.run {}
    }

    private func route() -> PlaybackRoute {
      PlaybackRoute(url: Self.clip, headers: [:], generation: nil, hls: false, looping: true)
    }

    @Test
    func aMutedPlayingSlotNeverTouchesTheSession() async {
      let port = FakeAudioSessionPort()
      let audio = session(.duck, port)
      let slot = await makeSlot(startMuted: true, session: audio)

      await slot.load(key: "a", route: route())
      await slot.play()
      await drain(audio)

      #expect(port.log.isEmpty)
      await slot.unload()
    }

    @Test
    func anUnmutedPlayingSlotTakesTheFocusAndPausingReleasesIt() async {
      let port = FakeAudioSessionPort()
      let slot = await makeSlot(startMuted: false, session: session(.mix, port))

      await slot.load(key: "a", route: route())
      await slot.play()
      await eventually("focus taken") { port.log.contains("active true") }

      await slot.pause()
      await eventually("focus released") { port.log.last == "active false" }
      #expect(port.log.filter { $0.hasPrefix("category") } == [FakeAudioSessionPort.category(.playback(.mix))])
      #expect(port.log.first == FakeAudioSessionPort.category(.playback(.mix)) && port.log.last == "active false")
      await slot.unload()
    }

    @Test
    func unmutingAPlayingSlotTakesTheFocusAndMutingReleasesIt() async {
      let port = FakeAudioSessionPort()
      let audio = session(.duck, port)
      let slot = await makeSlot(startMuted: true, session: audio)

      await slot.load(key: "a", route: route())
      await slot.play()
      await drain(audio)
      #expect(port.log.isEmpty)
      await slot.unmute()
      await eventually("focus taken") { port.log.contains("active true") }

      await slot.mute()
      await eventually("focus released") { port.log.last == "active false" }
      await slot.unload()
    }

    @Test
    func unloadingAnAudibleSlotReleasesTheFocus() async {
      let port = FakeAudioSessionPort()
      let slot = await makeSlot(startMuted: false, session: session(.mix, port))
      await slot.load(key: "a", route: route())
      await slot.play()
      await eventually("focus taken") { port.log.contains("active true") }

      await slot.unload()

      await eventually("focus released") { port.log.last == "active false" }
    }
  }
}
