@preconcurrency import AVFoundation
import CoreMedia
import Foundation
import MediaLayer

protocol VoicePCMSink: AnyObject, Sendable {
  func pcm(generation: UInt64, _ buffer: CMSampleBuffer)
  func lost(generation: UInt64, error: Error?)
  /// The source dropped a buffer: the sound has a hole.
  func dropped(generation: UInt64)
}

/// The native PCM boundary of a voice take, driven by the input owner of the audio core one generation at a time.
protocol VoicePCMSource: AnyObject, Sendable {
  func start(generation: UInt64, sink: VoicePCMSink)
  func stop(generation: UInt64)
}

/// Lets the input owner of the core drive a PCM source. Only the generation the owner started is heard: an older one reaches neither the take nor the owner.
final class VoiceMicrophone: CameraMicrophone, VoicePCMSink, @unchecked Sendable {
  private let source: VoicePCMSource
  private let lock = NSLock()
  private weak var listener: CameraMicrophoneListener?
  private var active: UInt64?
  private var take: VoicePCMSink?

  init(source: VoicePCMSource) { self.source = source }

  func attach(_ take: VoicePCMSink?) { lock.withLock { self.take = take } }

  func start(generation: UInt64, listener: CameraMicrophoneListener) {
    lock.withLock {
      self.listener = listener
      active = generation
    }
    source.start(generation: generation, sink: self)
  }

  func stop(generation: UInt64) {
    lock.withLock { if active == generation { active = nil } }
    source.stop(generation: generation)
  }

  func pcm(generation: UInt64, _ buffer: CMSampleBuffer) {
    guard isLivePCM(buffer) else { return }
    guard let (take, listener) = lock.withLock({ () -> (VoicePCMSink?, CameraMicrophoneListener?)? in
      active == generation ? (take, self.listener) : nil
    }) else { return }
    take?.pcm(generation: generation, buffer)
    listener?.microphoneHeard(generation: generation)
  }

  func dropped(generation: UInt64) {
    let take = lock.withLock { active == generation ? self.take : nil }
    take?.dropped(generation: generation)
  }

  func lost(generation: UInt64, error: Error?) {
    guard let (take, listener) = lock.withLock({ () -> (VoicePCMSink?, CameraMicrophoneListener?)? in
      active == generation ? (take, self.listener) : nil
    }) else { return }
    take?.lost(generation: generation, error: error)
    listener?.microphoneLost(generation: generation, error: error)
  }
}

/// The system microphone as a PCM source: an audio-only capture session on its own queue. The audio session belongs to the audio core.
final class AVCaptureVoiceSource: NSObject, VoicePCMSource, AVCaptureAudioDataOutputSampleBufferDelegate, @unchecked Sendable {
  private let queue = DispatchQueue(label: "com.noma4i.medialayer.voice.source", qos: .userInitiated)
  private let session = AVCaptureSession()
  private var sink: VoicePCMSink?
  private var generation: UInt64 = 0
  private var input: AVCaptureDeviceInput?
  private var output: AVCaptureAudioDataOutput?
  private var tokens: [NSObjectProtocol] = []

  override init() {
    super.init()
#if os(iOS)
    session.automaticallyConfiguresApplicationAudioSession = false
#endif
  }

  func start(generation: UInt64, sink: VoicePCMSink) {
    queue.async { [self] in
      teardown()
      self.generation = generation
      self.sink = sink
      if AVCaptureDevice.authorizationStatus(for: .audio) == .authorized { begin() }
      else { fail(NSError(domain: AVFoundationErrorDomain, code: AVError.applicationIsNotAuthorizedToUseDevice.rawValue)) }
    }
  }

  func stop(generation: UInt64) {
    queue.async { [self] in
      guard self.generation == generation else { return }
      teardown()
    }
  }

  private func begin() {
    do {
      guard let device = AVCaptureDevice.default(for: .audio) else { throw NSError(domain: AVFoundationErrorDomain, code: AVError.deviceNotConnected.rawValue) }
      let input = try AVCaptureDeviceInput(device: device)
      let output = AVCaptureAudioDataOutput()
      guard session.canAddInput(input), session.canAddOutput(output) else { throw NSError(domain: AVFoundationErrorDomain, code: AVError.sessionConfigurationChanged.rawValue) }
      output.setSampleBufferDelegate(self, queue: queue)
      session.beginConfiguration()
      session.addInput(input)
      session.addOutput(output)
      session.commitConfiguration()
      self.input = input
      self.output = output
      observe()
      session.startRunning()
    } catch {
      fail(error)
    }
  }

  private func observe() {
    let center = NotificationCenter.default
    let current = generation
    tokens = [
      center.addObserver(forName: AVCaptureSession.wasInterruptedNotification, object: session, queue: nil) { [weak self] _ in
        self?.queue.async { self?.lose(current, nil) }
      },
      center.addObserver(forName: AVCaptureSession.runtimeErrorNotification, object: session, queue: nil) { [weak self] note in
        let error: Error = (note.userInfo?[AVCaptureSessionErrorKey] as? Error) ?? NSError(domain: AVFoundationErrorDomain, code: AVError.unknown.rawValue)
        self?.queue.async { self?.lose(current, error) }
      }
    ]
  }

  private func fail(_ error: Error) { lose(generation, error) }

  private func lose(_ lost: UInt64, _ error: Error?) {
    guard generation == lost, let sink else { return }
    sink.lost(generation: lost, error: error)
  }

  private func teardown() {
    tokens.forEach(NotificationCenter.default.removeObserver)
    tokens = []
    if session.isRunning { session.stopRunning() }
    if input != nil || output != nil {
      session.beginConfiguration()
      if let output { session.removeOutput(output) }
      if let input { session.removeInput(input) }
      session.commitConfiguration()
    }
    output?.setSampleBufferDelegate(nil, queue: nil)
    input = nil
    output = nil
    sink = nil
  }

  func captureOutput(_ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection) {
    guard output === self.output, let sink else { return }
    sink.pcm(generation: generation, sampleBuffer)
  }

  func captureOutput(_ output: AVCaptureOutput, didDrop sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection) {
    queue.async { [self] in
      guard output === self.output, let sink else { return }
      sink.dropped(generation: generation)
    }
  }
}
