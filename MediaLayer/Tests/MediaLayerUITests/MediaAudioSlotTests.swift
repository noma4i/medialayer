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

    private func slot(startMuted: Bool, session: MediaAudioSession) -> AVQueuePlayerSlot {
      let factory = AVQueuePlayerSlotFactory(startMuted: startMuted, audio: session, slots: LoadedSlots())
      factory.events.bind(PlayerEventLog())
      return factory.create(slot: 7) as! AVQueuePlayerSlot
    }

    private func drain() async {
      await MainActor.run {}
      try? await Task.sleep(for: .milliseconds(400))
      await MainActor.run {}
    }

    private func route() -> PlaybackRoute {
      PlaybackRoute(url: Self.clip, headers: [:], generation: nil, hls: false, looping: true)
    }

    @Test
    func aMutedPlayingSlotNeverTouchesTheSession() async {
      let port = FakeAudioSessionPort()
      let slot = slot(startMuted: true, session: session(.duck, port))

      slot.load(key: "a", route: route())
      slot.play()
      await drain()

      #expect(port.log.isEmpty)
      slot.unload()
    }

    @Test
    func anUnmutedPlayingSlotTakesTheFocusAndPausingReleasesIt() async {
      let port = FakeAudioSessionPort()
      let slot = slot(startMuted: false, session: session(.mix, port))

      slot.load(key: "a", route: route())
      slot.play()
      await eventually("focus taken") { port.log.contains("active true") }

      slot.pause()
      await eventually("focus released") { port.log.last == "active false" }
      #expect(port.log == [FakeAudioSessionPort.category(.playback(.mix)), "active true", "active false"])
      slot.unload()
    }

    @Test
    func unmutingAPlayingSlotTakesTheFocusAndMutingReleasesIt() async {
      let port = FakeAudioSessionPort()
      let slot = slot(startMuted: true, session: session(.duck, port))

      slot.load(key: "a", route: route())
      slot.play()
      await drain()
      #expect(port.log.isEmpty)
      slot.unmute()
      await eventually("focus taken") { port.log.contains("active true") }

      slot.mute()
      await eventually("focus released") { port.log.last == "active false" }
      slot.unload()
    }

    @Test
    func unloadingAnAudibleSlotReleasesTheFocus() async {
      let port = FakeAudioSessionPort()
      let slot = slot(startMuted: false, session: session(.mix, port))
      slot.load(key: "a", route: route())
      slot.play()
      await eventually("focus taken") { port.log.contains("active true") }

      slot.unload()

      await eventually("focus released") { port.log.last == "active false" }
    }
  }
}
