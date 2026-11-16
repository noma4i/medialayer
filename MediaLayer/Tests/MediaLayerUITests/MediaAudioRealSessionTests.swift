@preconcurrency import AVFoundation
import MediaLayer
import Testing
@testable import MediaLayerUI

extension GlobalMedia {
  @Suite(.serialized)
  struct MediaAudioRealSessionTests {
    private func makeLink(_ focus: AudioFocus) throws -> (MediaLayer, AudioLink) {
      let core = try MediaLayer.open(config: OpenConfig(
        dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
        fetcher: TestFetcher(), settings: QuietSettings(), slots: nil,
        device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
        defaults: defaultLimits(), pool: defaultPoolSettings(), audioFocus: focus
      ))
      return (core, AudioLink(core: core, ports: AudioPorts(session: SystemAudioSession(focus: focus), calls: FakeCallObserver(), permission: GrantedMicrophone())))
    }

    @Test
    func theRealSessionReceivesTheCategoryAndOptionsOfEachPolicy() async throws {
      let real = AVAudioSession.sharedInstance()
      for (focus, mix, duck) in [(AudioFocus.exclusive, false, false), (.mix, true, false), (.duck, true, true)] {
        let (core, link) = try makeLink(focus)
        link.audible(slot: 1, true)
        await eventually("playback category") { real.category == .playback && real.categoryOptions.contains(.mixWithOthers) == mix && real.categoryOptions.contains(.duckOthers) == duck }

        #expect(real.category == .playback)
        #expect(real.mode == .moviePlayback)
        #expect(real.categoryOptions.contains(.mixWithOthers) == mix)
        #expect(real.categoryOptions.contains(.duckOthers) == duck)

        link.audible(slot: 1, false)
        link.close()
        core.close()
      }
    }

    @Test
    func anIdleSessionLeavesTheCategoryOfTheAppAlone() async throws {
      let real = AVAudioSession.sharedInstance()
      try real.setCategory(.playAndRecord, mode: .default, options: [.defaultToSpeaker])

      let (core, link) = try makeLink(.duck)
      defer { link.close(); core.close() }
      link.audible(slot: 1, false)

      #expect(real.category == .playAndRecord)
      #expect(real.categoryOptions == [.defaultToSpeaker])
    }

    @Test
    func recordingSwitchesTheRealSessionToTheMicrophoneCategory() async throws {
      let real = AVAudioSession.sharedInstance()
      let (core, link) = try makeLink(.mix)
      defer { link.close(); core.close() }

      let result = await link.admit(.voice)
      guard case let .success(hold) = result else { Issue.record("recording refused"); return }
      #expect(real.category == .playAndRecord)
      #expect(real.mode == .default)
      #expect(real.categoryOptions.contains(.allowBluetoothA2DP))
      #expect(!real.categoryOptions.contains(.defaultToSpeaker))
      #expect(!real.categoryOptions.contains(.mixWithOthers))

      hold.end()
      await eventually("restored playback category") { real.category == .playback && real.categoryOptions.contains(.mixWithOthers) }
      #expect(real.category == .playback)
      #expect(real.categoryOptions.contains(.mixWithOthers))
      link.audible(slot: 1, true)
      await eventually("playback category") { real.category == .playback && real.categoryOptions.contains(.mixWithOthers) }
      #expect(real.category == .playback)
      #expect(real.categoryOptions.contains(.mixWithOthers))
      link.audible(slot: 1, false)
    }
  }
}
