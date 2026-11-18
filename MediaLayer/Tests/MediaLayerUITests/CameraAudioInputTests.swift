#if os(iOS)
import AVFoundation
import CoreMedia
import Foundation
import MediaLayer
import MediaLayerTesting
import Testing
@testable import MediaLayerUI

private final class FakeMicrophone: CameraMicrophone, @unchecked Sendable {
  private let lock = NSLock()
  private var started: [UInt64] = []
  private var stopped: [UInt64] = []
  private var listener: CameraMicrophoneListener?
  private var failure: Error?
  private var failures = 0

  var starts: [UInt64] { lock.withLock { started } }
  var stops: [UInt64] { lock.withLock { stopped } }

  func failNextStart(_ error: Error, times: Int = 1) { lock.withLock { failure = error; failures = times } }

  func start(generation: UInt64, listener: CameraMicrophoneListener) {
    let failed = lock.withLock { () -> Error? in
      started.append(generation)
      self.listener = listener
      guard failures > 0 else { return nil }
      failures -= 1
      return failure
    }
    if let failed { DispatchQueue.global().async { listener.microphoneLost(generation: generation, error: failed) } }
    TestSignal.shared.fire()
  }

  func stop(generation: UInt64) {
    lock.withLock { stopped.append(generation) }
    TestSignal.shared.fire()
  }

  func hear(_ generation: UInt64) { lock.withLock { listener }?.microphoneHeard(generation: generation) }
  func lose(_ generation: UInt64, _ error: Error? = nil) { lock.withLock { listener }?.microphoneLost(generation: generation, error: error) }
}

private final class Issues: @unchecked Sendable {
  private let lock = NSLock()
  private var seen: [CameraRefusal?] = []
  var values: [CameraRefusal?] { lock.withLock { seen } }
  func add(_ value: CameraRefusal?) { lock.withLock { seen.append(value) }; TestSignal.shared.fire() }
}

extension GlobalMedia {
  @Suite(.serialized, .timeLimit(.minutes(1)))
  struct CameraAudioInputTests {
    private let microphone = FakeMicrophone()
    private let issues = Issues()
    private let retry = ManualTimer()

    private func open(_ rig: AudioRig) -> CameraAudioInput {
      let issues = issues
      return CameraAudioInput(link: rig.link, microphone: microphone, timer: retry.timer) { issues.add($0) }
    }

    private func connected(_ rig: AudioRig, generation: UInt64 = 1) async {
      await eventually("started \(generation)") { microphone.starts.last == generation }
      microphone.hear(generation)
      await eventually("heard \(generation)") { rig.availability.last == .available }
    }

    @Test
    func theSessionConnectsTheMicrophoneAndTheFirstSoundMakesItAvailable() async throws {
      let rig = try AudioRig()
      let input = open(rig)
      defer { input.close() }

      await eventually("connected") { microphone.starts == [1] }
      _ = await rig.link.readVolume()
      #expect(!rig.availability.contains(.noSound), "an input not heard yet does not blink no sound")
      microphone.hear(1)
      microphone.hear(1)

      _ = await rig.link.readVolume()
      #expect(rig.availability == [.available])
      #expect(issues.values == [nil])
    }

    @Test(arguments: ["own call", "external call"])
    func aCallStopsTheMicrophoneAndItsEndRestoresItWithoutATake(cause: String) async throws {
      let rig = try AudioRig()
      let input = open(rig)
      defer { input.close() }
      await connected(rig)
      let call = cause == "own call" ? try await rig.link.call(.unmanaged) : nil
      if call == nil { rig.calls.send(["external"]) }

      await eventually("stopped for the call") { microphone.stops == [1] }
      await eventually("shown") { rig.availability.last == (call == nil ? .externalCall : .ownCall) }
      #expect(microphone.starts == [1], "no reconnect before the call ends")

      if let call { call.end() } else { rig.calls.send([]) }
      await eventually("restored") { microphone.starts == [1, 2] && rig.availability.last == .available }
      microphone.hear(1)
      microphone.hear(2)
      await eventually("sound of the new generation") { rig.availability.last == .available }
    }

    @Test
    func aLossAfterTheFirstSoundIsNoSoundAndTheCoreRestoresAFreshGeneration() async throws {
      let rig = try AudioRig()
      let input = open(rig)
      defer { input.close() }
      await connected(rig)
      microphone.hear(1)

      microphone.lose(1)

      await eventually("lost") { rig.availability.contains(.noSound) && rig.availability.last == .noSound && microphone.starts == [1, 2] }
      #expect(microphone.stops.contains(1))
      microphone.hear(1)
      microphone.hear(2)
      await eventually("back") { rig.availability.last == .available }
      #expect(rig.availability == [.available, .noSound, .available], "only the loss the platform told is no sound")
    }

    @Test
    func aFailedStartIsATypedFailureThatWaitsForACauseInsteadOfLooping() async throws {
      let rig = try AudioRig()
      let failures = Collected(Media.failures)
      let busy = NSError(domain: AVFoundationErrorDomain, code: AVError.deviceInUseByAnotherApplication.rawValue)
      microphone.failNextStart(busy, times: MicrophoneRetry.attempts)
      let input = open(rig)
      defer { input.close() }
      for attempt in 1..<MicrophoneRetry.attempts {
        await eventually("the pause after attempt \(attempt)") { microphone.starts.count == attempt && retry.armed(MicrophoneRetry.pauseMs) }
        #expect(issues.values.isEmpty, "a retried failure is told to nobody")
        retry.fire(MicrophoneRetry.pauseMs)
      }

      await eventually("the failure reached the app") { failures.values.contains { $0.source == .camera } }
      let failure = try #require(failures.values.first { $0.source == .camera })
      #expect((failure.domain, failure.code) == (AVFoundationErrorDomain, Int64(AVError.deviceInUseByAnotherApplication.rawValue)))
      #expect(issues.values == [.microphoneBusy])
      _ = await rig.link.readVolume()
      #expect(microphone.starts == [1, 2, 3])
      #expect(rig.availability.last == .noSound)

      rig.session.send(.routeChanged(.newDeviceAvailable, hadPrivateOutput: false))

      await eventually("the route restores it") { microphone.starts == [1, 2, 3, 4] }
      microphone.hear(4)
      await eventually("available") { rig.availability.last == .available }
      #expect(issues.values == [.microphoneBusy, nil])
    }

    @Test
    func aClosedInputStopsTheMicrophoneAndHearsNothingMore() async throws {
      let rig = try AudioRig()
      let input = open(rig)
      await connected(rig)

      input.close()
      input.close()

      await eventually("stopped") { microphone.stops == [1] }
      microphone.hear(1)
      microphone.lose(1)
      _ = await rig.link.readVolume()
      #expect(microphone.starts == [1])
      #expect(microphone.stops == [1])
      #expect(rig.availability.last == .available)
    }

    @Test
    func theClosedLayerStopsTheMicrophoneAndEndsTheAvailability() async throws {
      let rig = try AudioRig()
      let input = open(rig)
      defer { input.close() }
      let pushed = Collected(rig.link.availability)
      await connected(rig)

      await rig.link.awaitClose()

      #expect(microphone.stops == [1])
      await eventually("ended") { pushed.finished }
      #expect(pushed.values.last == .available)
    }

    @Test
    func inv_audio_85_aVoiceTicketTakesTheMicrophoneOfTheIdleCameraInputAndGivesItBackAtItsEnd() async throws {
      let rig = try AudioRig()
      let input = open(rig)
      await connected(rig)
      guard case let .success(camera) = await rig.link.admit(.video), case let .success(voice) = await rig.link.admit(.voice) else {
        Issue.record("both tickets are admitted")
        return
      }

      await eventually("the camera input gave its microphone to the voice") { microphone.stops == [1] }
      #expect(rig.availability.last == .available, "a yielded camera input is no missing sound")

      camera.end()
      voice.end()
      await connected(rig, generation: 2)
      #expect(microphone.stops == [1], "the camera input is back after the voice and lives between takes")

      input.close()
      await eventually("the input and the session are released") { microphone.stops == [1, 2] && rig.session.log.last == "deactivate" }
    }

    @Test
    func onlyAValidBufferWithSamplesIsSound() throws {
      let buffer = try #require(CameraTakeTests.Fixture.make().audio.first)
      #expect(isLivePCM(buffer))
      CMSampleBufferInvalidate(buffer)
      #expect(!isLivePCM(buffer))
    }

    @Test
    func theRefusalOfAMicrophoneKeepsItsEarlierType() {
      let table: [(Int, CameraRefusal)] = [
        (AVError.applicationIsNotAuthorizedToUseDevice.rawValue, .permissionDenied),
        (AVError.deviceNotConnected.rawValue, .microphoneBusy),
        (AVError.deviceAlreadyUsedByAnotherSession.rawValue, .microphoneBusy),
        (AVError.sessionHardwareCostOverage.rawValue, .microphoneBusy),
        (AVError.sessionConfigurationChanged.rawValue, .configureFailed),
      ]
      for (code, reason) in table {
        #expect(microphoneRefusal(NSError(domain: AVFoundationErrorDomain, code: code)) == reason, "\(code)")
      }
      #expect(microphoneRefusal(NSError(domain: NSOSStatusErrorDomain, code: 1)) == .configureFailed)
    }
  }
}
#endif
