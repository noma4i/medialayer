import AVFoundation
import Foundation
import MediaLayer
import Testing
@testable import MediaLayerUI

final class TestSessionPort: AudioSessionPort, @unchecked Sendable {
  let base: FakeSessionPort
  private let lock = NSLock()
  private var applied: [SessionPlan] = []
  var plans: [SessionPlan] { lock.withLock { applied } }
  init(_ base: FakeSessionPort) { self.base = base }
  func systemEvents() -> AsyncStream<SystemAudioEvent> { base.systemEvents() }
  func apply(_ plan: SessionPlan, activate: Bool) throws {
    lock.withLock { applied.append(plan) }
    try base.apply(plan, activate: activate)
  }
  func deactivate() throws { try base.deactivate() }
  func overrideSpeaker(_ on: Bool) throws { try base.overrideSpeaker(on) }
  func outputVolume() -> Float { base.outputVolume() }
  func volumeChanges() -> AsyncStream<Float> { base.volumeChanges() }
}

final class TestAudioRig: @unchecked Sendable {
  let core: MediaLayer
  let port: FakeSessionPort
  let link: AudioLink
  let audio: MediaAudioSession
  let checked: TestSessionPort
  var isRecording: Bool { port.log.last?.hasPrefix("apply AVAudioSessionCategoryPlayAndRecord") == true }

  init(focus: AudioFocus = .mix, port: FakeSessionPort = FakeSessionPort()) throws {
    self.port = port
    let checked = TestSessionPort(port)
    self.checked = checked
    let core = try MediaLayer.open(config: OpenConfig(
      dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
      fetcher: TestFetcher(), settings: QuietSettings(), slots: nil,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(), pool: defaultPoolSettings(), audioFocus: focus
    ))
    self.core = core
    let link = AudioLink(core: core, ports: AudioPorts(session: checked, calls: FakeCallObserver(), permission: GrantedMicrophone()))
    self.link = link
    let audio = MediaAudioSession()
    self.audio = audio
    audio.bind(link)
  }

  deinit { link.close(); core.close() }
}

private final class VolumeSessionPort: AudioSessionPort, @unchecked Sendable {
  private let lock = NSLock()
  private var entries: [String] = []
  private var applied: [SessionPlan] = []
  private var sink: AsyncStream<Float>.Continuation?
  private var ended = false
  var log: [String] { lock.withLock { entries } }
  var plans: [SessionPlan] { lock.withLock { applied } }
  var volumeStreamEnded: Bool { lock.withLock { ended } }

  func systemEvents() -> AsyncStream<SystemAudioEvent> { AsyncStream { _ in } }
  func apply(_ plan: SessionPlan, activate: Bool) throws {
    lock.withLock { applied.append(plan); entries.append("apply \(plan.category.rawValue) \(plan.mode.rawValue) activate=\(activate)") }
    TestSignal.shared.fire()
  }
  func deactivate() throws { lock.withLock { entries.append("deactivate") }; TestSignal.shared.fire() }
  func overrideSpeaker(_ on: Bool) throws {}
  func outputVolume() -> Float { 0.5 }
  func volumeChanges() -> AsyncStream<Float> {
    AsyncStream { continuation in
      lock.withLock { sink = continuation; ended = false }
      continuation.onTermination = { [weak self] _ in self?.lock.withLock { self?.ended = true }; TestSignal.shared.fire() }
    }
  }
  func pressVolume(_ value: Float) { lock.withLock { sink }?.yield(value) }
}

private final class VolumeRig {
  let core: MediaLayer
  let port: VolumeSessionPort
  let link: AudioLink
  let audio: MediaAudioSession

  init(focus: AudioFocus) throws {
    let port = VolumeSessionPort()
    self.port = port
    let core = try MediaLayer.open(config: OpenConfig(
      dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
      fetcher: TestFetcher(), settings: QuietSettings(), slots: nil,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(), pool: defaultPoolSettings(), audioFocus: focus
    ))
    self.core = core
    let link = AudioLink(core: core, ports: AudioPorts(session: port, calls: FakeCallObserver(), permission: GrantedMicrophone()))
    self.link = link
    let audio = MediaAudioSession()
    self.audio = audio
    audio.bind(link)
  }
  deinit { link.close(); core.close() }
}

private final class RaiseCount: @unchecked Sendable {
  private let lock = NSLock()
  private var count = 0
  var value: Int { lock.withLock { count } }
  func add() { lock.withLock { count += 1 }; TestSignal.shared.fire() }
}

extension GlobalMedia {
  @Suite(.serialized, .timeLimit(.minutes(1)))
  struct MediaAudioSlotTests {
    private func readOnePress(_ session: MediaAudioSession, _ port: VolumeSessionPort) async -> (Float, Float?) {
      guard let reading = await session.readVolume() else { return (-1, nil) }
      port.pressVolume(0.7)
      var first: Float?
      for await volume in reading.changes { first = volume; break }
      return (reading.initial, first)
    }

    @Test
    func readingTheVolumeHoldsANonDuckingSessionUntilTheStreamEnds() async throws {
      let rig = try VolumeRig(focus: .duck)
      let port = rig.port
      let (initial, pressed) = await readOnePress(rig.audio, port)
      #expect(initial == 0.5)
      #expect(pressed == 0.7)
      #expect(port.log.prefix(1) == ["apply AVAudioSessionCategoryPlayback AVAudioSessionModeMoviePlayback activate=true"])
      #expect(port.plans.first == SessionPlan.of(.playback(focus: .mix)))
      await eventually("session released") { port.log.last == "deactivate" }
      await eventually("port stream ended") { port.volumeStreamEnded }
    }

    @Test
    func inv_audio_10_volumeRaisesEmitsOnARiseOnlyAndEndingTheStreamRemovesTheObserver() async throws {
      let rig = try VolumeRig(focus: .mix)
      let port = rig.port
      let raises = await rig.audio.volumeRaises()
      let count = RaiseCount()
      let collector = Task { for await _ in raises { count.add() } }
      port.pressVolume(0.7)
      await eventually("a rise reported") { count.value == 1 }
      port.pressVolume(0.3)
      port.pressVolume(0.3005)
      port.pressVolume(0.5)
      await eventually("the next rise reported") { count.value >= 2 }
      #expect(count.value == 2)
      #expect(!port.volumeStreamEnded)
      collector.cancel()
      await eventually("port stream ended") { port.volumeStreamEnded }
    }

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
      #expect(rig.checked.plans == [SessionPlan.of(.playback(focus: .mix))])
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
