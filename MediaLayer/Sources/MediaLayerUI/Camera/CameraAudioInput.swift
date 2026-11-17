#if os(iOS)
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
}

enum CaptureAudio {
  case managed
  case native(CameraMicrophone)
}

func isLivePCM(_ buffer: CMSampleBuffer) -> Bool {
  CMSampleBufferIsValid(buffer) && CMSampleBufferDataIsReady(buffer) && CMSampleBufferGetNumSamples(buffer) > 0
}

func microphoneRefusal(_ error: Error) -> CameraRefusal {
  let native = error as NSError
  guard native.domain == AVFoundationErrorDomain else { return .configureFailed }
  switch native.code {
  case AVError.applicationIsNotAuthorizedToUseDevice.rawValue: return .permissionDenied
  case AVError.deviceNotConnected.rawValue, AVError.deviceInUseByAnotherApplication.rawValue, AVError.deviceAlreadyUsedByAnotherSession.rawValue, AVError.sessionHardwareCostOverage.rawValue: return .microphoneBusy
  default: return .configureFailed
  }
}

/// The owner of the camera microphone in the audio core. It lives between takes and every word of the native microphone (first sound, loss, failure, close) is told by one generation: a word of an ended generation reaches neither the core nor the take.
final class CameraAudioInput: AudioInputOwner, CameraMicrophoneListener, @unchecked Sendable {
  private struct Live {
    let core: UInt64
    let audio: UInt64
    var heard = false
  }

  private let lock = NSLock()
  private let microphone: CameraMicrophone
  private let issue: @Sendable (CameraRefusal?) -> Void
  private var live: Live?
  private var input: AudioInputHandle?
  private var opened: AudioInputHandle?
  private var audioGeneration: UInt64 = 0
  private var closed = false

  init(link: AudioLink, microphone: CameraMicrophone, issue: @escaping @Sendable (CameraRefusal?) -> Void) {
    self.microphone = microphone
    self.issue = issue
    opened = link.openInput(.video, owner: self)
  }

  func restore(_ input: AudioInputHandle, generation: UInt64) {
    lock.withLock {
      guard !closed else { return }
      self.input = input
      if let live { microphone.stop(generation: live.audio) }
      audioGeneration += 1
      live = Live(core: generation, audio: audioGeneration)
      microphone.start(generation: audioGeneration, listener: self)
    }
  }

  func stop(generation: UInt64) {
    lock.withLock {
      guard let current = live, current.core == generation else { return }
      live = nil
      microphone.stop(generation: current.audio)
    }
  }

  func microphoneHeard(generation: UInt64) {
    lock.withLock {
      guard var current = live, current.audio == generation, !current.heard, let input else { return }
      current.heard = true
      live = current
      issue(nil)
      input.sound(generation: current.core, heard: true)
    }
  }

  func microphoneLost(generation: UInt64, error: Error?) {
    lock.withLock {
      guard let current = live, current.audio == generation, let input else { return }
      live = nil
      microphone.stop(generation: current.audio)
      guard let error else { return input.sound(generation: current.core, heard: false) }
      Media.recordNativeFailure(error, source: .camera)
      issue(microphoneRefusal(error))
      input.restoreFailed(generation: current.core)
    }
  }

  func close() {
    let handle = lock.withLock { () -> AudioInputHandle? in
      guard !closed else { return nil }
      closed = true
      if let live { microphone.stop(generation: live.audio) }
      live = nil
      return opened
    }
    handle?.close()
  }

  deinit { close() }
}
#endif
