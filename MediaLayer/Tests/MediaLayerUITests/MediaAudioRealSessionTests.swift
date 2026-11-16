@preconcurrency import AVFoundation
import MediaLayer
import MediaLayerTesting
import Testing
@testable import MediaLayerUI

private final class RealSessionPort: AudioSessionPort, @unchecked Sendable {
  let session: SystemAudioSession
  private let lock = NSLock()
  private var releases = 0
  var releaseCount: Int { lock.withLock { releases } }
  init(session: SystemAudioSession) { self.session = session }
  func systemEvents(_ sink: @escaping @Sendable (SystemAudioEvent) -> Void) -> @Sendable () -> Void { session.systemEvents(sink) }
  func apply(_ plan: SessionPlan, activate: Bool) throws { defer { TestSignal.shared.fire() }; try session.apply(plan, activate: activate) }
  func deactivate() throws { try session.deactivate(); lock.withLock { releases += 1 }; TestSignal.shared.fire() }
  func overrideSpeaker(_ on: Bool) throws { try session.overrideSpeaker(on) }
  func outputVolume() -> Float { session.outputVolume() }
  func volumeChanges() -> AsyncStream<Float> { session.volumeChanges() }
}

extension GlobalMedia {
  @Suite(.serialized)
  struct MediaAudioRealSessionTests {
    private func makeLink(_ focus: AudioFocus) throws -> (MediaLayer, AudioLink, RealSessionPort) {
      let core = try MediaLayer.open(config: OpenConfig(
        dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
        fetcher: TestFetcher(), settings: QuietSettings(), slots: nil,
        device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
        defaults: defaultLimits(), pool: defaultPoolSettings(), audioFocus: focus
      ))
      let port = RealSessionPort(session: SystemAudioSession(focus: focus))
      return (core, AudioLink(core: core, ports: AudioPorts(session: port, calls: FakeCallObserver(), permission: GrantedMicrophone())), port)
    }

    @Test
    func theRealSessionReceivesTheCategoryAndOptionsOfEachPolicy() async throws {
      let real = AVAudioSession.sharedInstance()
      for (focus, mix, duck) in [(AudioFocus.exclusive, false, false), (.mix, true, false), (.duck, true, true)] {
        let (core, link, port) = try makeLink(focus)
        link.audible(slot: 1, true)
        await eventually("playback category") { real.category == .playback && real.categoryOptions.contains(.mixWithOthers) == mix && real.categoryOptions.contains(.duckOthers) == duck }

        #expect(real.category == .playback)
        #expect(real.mode == .moviePlayback)
        #expect(real.categoryOptions.contains(.mixWithOthers) == mix)
        #expect(real.categoryOptions.contains(.duckOthers) == duck)

        link.audible(slot: 1, false)
        await eventually("session released") { port.releaseCount == 1 }
        link.close()
        core.close()
      }
    }

    @Test
    func anIdleSessionLeavesTheCategoryOfTheAppAlone() async throws {
      let real = AVAudioSession.sharedInstance()
      try real.setCategory(.playAndRecord, mode: .default, options: [.defaultToSpeaker])

      let (core, link, _) = try makeLink(.duck)
      defer { link.close(); core.close() }
      link.audible(slot: 1, false)

      #expect(real.category == .playAndRecord)
      #expect(real.categoryOptions == [.defaultToSpeaker])
    }

    @Test
    func recordingSwitchesTheRealSessionToTheMicrophoneCategory() async throws {
      let real = AVAudioSession.sharedInstance()
      let (core, link, _) = try makeLink(.mix)
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
