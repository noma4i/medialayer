import AVFoundation
import Foundation
import MediaLayer
import Testing
@testable import MediaLayerUI

final class TestAudioRig: @unchecked Sendable {
  let core: MediaLayer
  let port: FakeSessionPort
  let link: AudioLink
  let audio: MediaAudioSession

  init(focus: AudioFocus = .mix, port: FakeSessionPort = FakeSessionPort()) throws {
    self.port = port
    core = try MediaLayer.open(config: OpenConfig(
      dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
      fetcher: TestFetcher(), settings: QuietSettings(), slots: nil,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(), pool: defaultPoolSettings(), audioFocus: focus
    ))
    link = AudioLink(core: core, ports: AudioPorts(session: port, calls: FakeCallObserver(), permission: GrantedMicrophone()))
    audio = MediaAudioSession()
    audio.bind(link)
  }

  deinit { link.close(); core.close() }
}

extension GlobalMedia {
  @Suite(.serialized, .timeLimit(.minutes(1)))
  struct MediaAudioSlotTests {
    private static var clip: String {
      Bundle.module.url(forResource: "silent", withExtension: "mp4", subdirectory: "Fixtures")!.absoluteString
    }

    private func makeSlot(startMuted: Bool, session: MediaAudioSession) async -> AVQueuePlayerSlot {
      let factory = AVQueuePlayerSlotFactory(startMuted: startMuted, audio: session, slots: LoadedSlots())
      factory.events.bind(PlayerEventLog())
      return await factory.makeSlot(7)
    }

    private func drain() async {
      await MainActor.run {}
      await MainActor.run {}
    }

    private func route() -> PlaybackRoute {
      PlaybackRoute(url: Self.clip, headers: [:], generation: nil, hls: false, looping: true)
    }

    @Test
    func aMutedPlayingSlotNeverTouchesTheSession() async throws {
      let rig = try TestAudioRig(focus: .duck)
      let port = rig.port
      let slot = await makeSlot(startMuted: true, session: rig.audio)

      await slot.load(key: "a", route: route())
      await slot.play()
      await drain()

      #expect(port.log.isEmpty)
      await slot.unload()
    }

    @Test
    func anUnmutedPlayingSlotTakesTheFocusAndPausingReleasesIt() async throws {
      let rig = try TestAudioRig()
      let port = rig.port
      let slot = await makeSlot(startMuted: false, session: rig.audio)

      await slot.load(key: "a", route: route())
      await slot.play()
      await eventually("focus taken") { port.log.contains("apply AVAudioSessionCategoryPlayback AVAudioSessionModeMoviePlayback activate=true") }

      await slot.pause()
      await eventually("focus released") { port.log.last == "deactivate" }
      #expect(port.log.filter { $0.hasPrefix("apply") } == ["apply AVAudioSessionCategoryPlayback AVAudioSessionModeMoviePlayback activate=true"])
      #expect(port.log.first == "apply AVAudioSessionCategoryPlayback AVAudioSessionModeMoviePlayback activate=true" && port.log.last == "deactivate")
      await slot.unload()
    }

    @Test
    func unmutingAPlayingSlotTakesTheFocusAndMutingReleasesIt() async throws {
      let rig = try TestAudioRig(focus: .duck)
      let port = rig.port
      let slot = await makeSlot(startMuted: true, session: rig.audio)

      await slot.load(key: "a", route: route())
      await slot.play()
      await drain()
      #expect(port.log.isEmpty)
      await slot.unmute()
      await eventually("focus taken") { port.log.contains("apply AVAudioSessionCategoryPlayback AVAudioSessionModeMoviePlayback activate=true") }

      await slot.mute()
      await eventually("focus released") { port.log.last == "deactivate" }
      await slot.unload()
    }

    @Test
    func unloadingAnAudibleSlotReleasesTheFocus() async throws {
      let rig = try TestAudioRig()
      let port = rig.port
      let slot = await makeSlot(startMuted: false, session: rig.audio)
      await slot.load(key: "a", route: route())
      await slot.play()
      await eventually("focus taken") { port.log.contains("apply AVAudioSessionCategoryPlayback AVAudioSessionModeMoviePlayback activate=true") }

      await slot.unload()

      await eventually("focus released") { port.log.last == "deactivate" }
    }
  }
}
