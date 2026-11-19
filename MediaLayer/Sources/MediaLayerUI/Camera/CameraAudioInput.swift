import AVFoundation
import CoreMedia
import Foundation
import MediaLayer

protocol CameraMicrophoneListener: AnyObject, Sendable {
  func microphoneHeard(generation: UInt64)
  func microphoneLost(generation: UInt64, error: Error?)
}

protocol CameraMicrophone: AnyObject, Sendable {
  func start(generation: UInt64, listener: CameraMicrophoneListener)
  func stop(generation: UInt64)
  /// The last attempt of a start gave no sound (owner 2026-11-19, `MicrophoneRetry`): the loss of a source before its first PCM is final only here.
  func gaveUp(generation: UInt64, error: Error?)
}

extension CameraMicrophone {
  func gaveUp(generation: UInt64, error: Error?) {}
}

func isLivePCM(_ buffer: CMSampleBuffer) -> Bool {
  CMSampleBufferIsValid(buffer) && CMSampleBufferDataIsReady(buffer) && CMSampleBufferGetNumSamples(buffer) > 0
}

func microphoneRefusal(_ error: Error) -> CameraRefusal {
  let native = error as NSError
  guard native.domain == AVFoundationErrorDomain else { return .configureFailed }
  switch native.code {
  case AVError.applicationIsNotAuthorizedToUseDevice.rawValue: return .permissionDenied
  case AVError.deviceNotConnected.rawValue, AVError.deviceInUseByAnotherApplication.rawValue, AVError.deviceAlreadyUsedByAnotherSession.rawValue: return .microphoneBusy
#if os(iOS)
  case AVError.sessionHardwareCostOverage.rawValue: return .microphoneBusy
#endif
  default: return .configureFailed
  }
}

/// The owner of the camera microphone in the audio core. It lives between takes and every word of the native microphone (first sound, loss, failure, close) is told by one generation: a word of an ended generation reaches neither the core nor the take.
/// A restore runs the attempts of `MicrophoneRetry` (owner 2026-11-19): a source lost or silent before its first sound is stopped and started again after the pause, told to nobody; only the last attempt's outcome reaches the core and the take.
final class CameraAudioInput: AudioInputOwner, CameraMicrophoneListener, @unchecked Sendable {
  /// The attempts of one core generation.
  private final class Run {
    let core: UInt64
    var attempt = 0
    init(core: UInt64) { self.core = core }
  }

  private struct Live {
    let run: Run
    let audio: UInt64
    var heard = false
  }

  private let lock = NSLock()
  private let microphone: CameraMicrophone
  private let issue: @Sendable (CameraRefusal?) -> Void
  private let failureSource: FailureSource?
  private let timer: RetryTimer
  private var live: Live?
  /// The run waiting the pause before its next attempt.
  private var pause: Run?
  private var cancelClock: (@Sendable () -> Void)?
  /// The core generation of the last run that ended silent: a start of a take runs the attempts again (`retry`).
  private var silent: UInt64?
  private var input: AudioInputHandle?
  private var opened: AudioInputHandle?
  private var audioGeneration: UInt64 = 0
  private var closed = false

  /// `kind` and `failureSource` default to the camera. A voice take opens the same owner as `.voice` with no failure source: its recorder owns the one event of a failure.
  init(link: AudioLink, microphone: CameraMicrophone, kind: RecordingKind = .video, failureSource: FailureSource? = .camera, timer: RetryTimer = .system, issue: @escaping @Sendable (CameraRefusal?) -> Void) {
    self.microphone = microphone
    self.issue = issue
    self.failureSource = failureSource
    self.timer = timer
    opened = link.openInput(kind, owner: self)
  }

  func restore(_ input: AudioInputHandle, generation: UInt64) {
    lock.withLock {
      guard !closed else { return }
      self.input = input
      release()
      attempt(Run(core: generation))
    }
  }

  /// A start of a take retries a microphone whose last run ended silent (owner 2026-11-19): a fresh run of attempts on the same core generation, told to nobody.
  func retry() {
    lock.withLock {
      guard !closed, let core = silent, input != nil else { return }
      release()
      attempt(Run(core: core))
    }
  }

  func stop(generation: UInt64) {
    lock.withLock {
      guard (live?.run ?? pause)?.core == generation else { return }
      release()
    }
  }

  func microphoneHeard(generation: UInt64) {
    lock.withLock {
      guard var current = live, current.audio == generation, !current.heard, let input else { return }
      current.heard = true
      live = current
      silent = nil
      stopClock()
      issue(nil)
      input.sound(generation: current.run.core, heard: true)
    }
  }

  func microphoneLost(generation: UInt64, error: Error?) {
    let gaveUp = lock.withLock { () -> Bool in
      guard let current = live, current.audio == generation, let input else { return false }
      let refusal = error.map(microphoneRefusal)
      if refusal != .permissionDenied, retry(current) { return false }
      release()
      guard let error else {
        silent = current.run.core
        if !current.heard { issue(.microphoneSilent) }
        input.sound(generation: current.run.core, heard: false)
        return !current.heard
      }
      if let failureSource { Media.recordNativeFailure(error, source: failureSource) }
      issue(refusal)
      input.restoreFailed(generation: current.run.core)
      return !current.heard
    }
    // Out of the lock: the take that hears it closes this owner.
    if gaveUp { microphone.gaveUp(generation: generation, error: error) }
  }

  /// Starts the next attempt of `run` and its wait for the first sound. Runs under the lock.
  private func attempt(_ run: Run) {
    run.attempt += 1
    audioGeneration += 1
    let audio = audioGeneration
    live = Live(run: run, audio: audio)
    cancelClock = timer.schedule(MicrophoneRetry.soundWaitMs) { [weak self] in self?.soundWaitEnded(audio) }
    microphone.start(generation: audio, listener: self)
  }

  private func soundWaitEnded(_ audio: UInt64) {
    let gaveUp = lock.withLock { () -> Bool in
      guard let current = live, current.audio == audio, !current.heard, let input else { return false }
      cancelClock = nil
      if retry(current) { return false }
      // The last attempt gave no sound in time: the same word as a source the system silenced.
      silent = current.run.core
      issue(.microphoneSilent)
      input.sound(generation: current.run.core, heard: false)
      return true
    }
    // Out of the lock: the take that hears it closes this owner.
    if gaveUp { microphone.gaveUp(generation: audio, error: nil) }
  }

  /// The source of `current` gave no sound: true when another attempt follows after the pause. Runs under the lock.
  private func retry(_ current: Live) -> Bool {
    guard !current.heard, current.run.attempt < MicrophoneRetry.attempts else { return false }
    stopClock()
    live = nil
    microphone.stop(generation: current.audio)
    let run = current.run
    pause = run
    cancelClock = timer.schedule(MicrophoneRetry.pauseMs) { [weak self] in self?.resume(run) }
    return true
  }

  private func resume(_ run: Run) {
    lock.withLock {
      guard !closed, pause === run else { return }
      pause = nil
      cancelClock = nil
      attempt(run)
    }
  }

  private func stopClock() {
    cancelClock?()
    cancelClock = nil
  }

  /// Ends the run: its clock, its pause and its source. Runs under the lock.
  private func release() {
    silent = nil
    stopClock()
    pause = nil
    if let live { microphone.stop(generation: live.audio) }
    live = nil
  }

  func close() {
    let handle = lock.withLock { () -> AudioInputHandle? in
      guard !closed else { return nil }
      closed = true
      release()
      return opened
    }
    handle?.close()
  }

  deinit { close() }
}
