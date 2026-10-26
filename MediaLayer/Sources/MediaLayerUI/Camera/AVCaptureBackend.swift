#if os(iOS)
@preconcurrency import AVFoundation
import CoreMedia
import Foundation
import MediaLayer

/// The hardware camera: one `AVCaptureSession` with the photo output or the movie output of the profile. Every call runs on its own serial queue, never on Main.
final class AVCaptureBackend: NSObject, CaptureBackend, @unchecked Sendable {
  let session = AVCaptureSession()
  private let queue = DispatchQueue(label: "com.noma4i.medialayer.camera.session")
  private let frames = DispatchQueue(label: "com.noma4i.medialayer.camera.frames")
  private let lock = NSLock()
  private var sink: (@Sendable (CaptureEvent) -> Void)?
  private var camera: AVCaptureDevice?
  private var photoOutput: AVCapturePhotoOutput?
  private var movieOutput: AVCaptureMovieFileOutput?
  private var frameOutput: AVCaptureVideoDataOutput?
  private var rotation: AVCaptureDevice.RotationCoordinator?
  private var recording: MovieDelegate?
  private var photoDelegates: [UUID: PhotoDelegate] = [:]
  private var tap: (@Sendable (CVPixelBuffer, UInt64) -> Void)?
  private var chosenPhotoSize: PixelSize?
  private var tokens: [NSObjectProtocol] = []

  override init() {
    super.init()
    session.automaticallyConfiguresApplicationAudioSession = false
    let center = NotificationCenter.default
    tokens = [
      center.addObserver(forName: AVCaptureSession.didStartRunningNotification, object: session, queue: nil) { [weak self] _ in self?.emit(.streaming(on: true)) },
      center.addObserver(forName: AVCaptureSession.didStopRunningNotification, object: session, queue: nil) { [weak self] _ in self?.emit(.streaming(on: false)) },
      center.addObserver(forName: AVCaptureSession.wasInterruptedNotification, object: session, queue: nil) { [weak self] _ in self?.emit(.streaming(on: false)) },
      center.addObserver(forName: AVCaptureSession.interruptionEndedNotification, object: session, queue: nil) { [weak self] _ in
        guard let self else { return }
        self.emit(.streaming(on: self.session.isRunning))
      },
      center.addObserver(forName: AVCaptureSession.runtimeErrorNotification, object: session, queue: nil) { [weak self] note in
        let error = note.userInfo?[AVCaptureSessionErrorKey] as? AVError
        self?.emit(.fault(kind: error?.code == .mediaServicesWereReset ? .reset : .other))
      }
    ]
  }

  deinit {
    tokens.forEach(NotificationCenter.default.removeObserver)
    session.stopRunning()
  }

  var photoSize: PixelSize? { lock.withLock { chosenPhotoSize } }

  func setSink(_ sink: @escaping @Sendable (CaptureEvent) -> Void) {
    lock.withLock { self.sink = sink }
  }

  private func emit(_ event: CaptureEvent) {
    let sink = lock.withLock { self.sink }
    sink?(event)
  }

  func bind(lens: CameraLens, profile: CaptureProfile) {
    queue.async { self.configure(lens: lens, profile: profile) }
  }

  private func authorized(_ type: AVMediaType) -> Bool {
    AVCaptureDevice.authorizationStatus(for: type) == .authorized
  }

  private func configure(lens: CameraLens, profile: CaptureProfile) {
    guard authorized(.video), !profile.audio || authorized(.audio) else { return emit(.refuse(reason: .permissionDenied)) }
    guard let device = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: lens == .front ? .front : .back) else {
      return emit(.refuse(reason: .noCamera))
    }
    let inputs: [AVCaptureDeviceInput]
    do {
      var made = [try AVCaptureDeviceInput(device: device)]
      if profile.audio {
        guard let microphone = AVCaptureDevice.default(for: .audio) else { return emit(.refuse(reason: .configureFailed)) }
        made.append(try AVCaptureDeviceInput(device: microphone))
      }
      inputs = made
    } catch {
      return emit(.refuse(reason: .busy))
    }
    let photo = profile.mode == .photo ? AVCapturePhotoOutput() : nil
    let movie = profile.mode == .video ? AVCaptureMovieFileOutput() : nil
    session.beginConfiguration()
    session.inputs.forEach(session.removeInput)
    session.outputs.forEach(session.removeOutput)
    frameOutput = nil
    var outputs: [AVCaptureOutput] = []
    if let photo { outputs.append(photo) }
    if let movie { outputs.append(movie) }
    let added = inputs.allSatisfy { session.canAddInput($0) } && outputs.allSatisfy { session.canAddOutput($0) }
    guard added else {
      session.commitConfiguration()
      return emit(.refuse(reason: .configureFailed))
    }
    inputs.forEach(session.addInput)
    outputs.forEach(session.addOutput)
    session.commitConfiguration()
    camera = device
    photoOutput = photo
    movieOutput = movie
    rotation = AVCaptureDevice.RotationCoordinator(device: device, previewLayer: nil)
    let choice = applyFormat(device, profile: profile)
    if let photo, let size = choice?.photoSize {
      photo.maxPhotoDimensions = CMVideoDimensions(width: Int32(size.width), height: Int32(size.height))
    }
    lock.withLock { chosenPhotoSize = choice?.photoSize }
    installFrameOutput()
    if !session.isRunning { session.startRunning() }
    let hasFlash = profile.mode == .photo ? device.isFlashAvailable : device.hasTorch
    emit(.opened(hasFlash: hasFlash, zoomMin: Float(device.minAvailableVideoZoomFactor), zoomMax: Float(device.maxAvailableVideoZoomFactor)))
    emit(.streaming(on: session.isRunning))
  }

  private func applyFormat(_ device: AVCaptureDevice, profile: CaptureProfile) -> FormatChoice? {
    let candidates = device.formats.map { format in
      let video = format.formatDescription.dimensions
      return FormatCandidate(
        videoSize: PixelSize(width: UInt32(video.width), height: UInt32(video.height)),
        photoSizes: format.supportedMaxPhotoDimensions.map { PixelSize(width: UInt32($0.width), height: UInt32($0.height)) },
        frameRates: format.videoSupportedFrameRateRanges.map { FpsRange(low: UInt32($0.minFrameRate), high: UInt32($0.maxFrameRate.rounded(.down))) },
        highPhotoQuality: format.isHighPhotoQualitySupported,
        highestPhotoQuality: format.isHighestPhotoQualitySupported
      )
    }
    guard let choice = pickFormat(candidates: candidates, profile: profile), (try? device.lockForConfiguration()) != nil else { return nil }
    device.activeFormat = device.formats[Int(choice.index)]
    let duration = CMTime(value: 1, timescale: CMTimeScale(profile.fps))
    let supported = device.activeFormat.videoSupportedFrameRateRanges.contains { $0.minFrameDuration <= duration && duration <= $0.maxFrameDuration }
    if supported {
      device.activeVideoMinFrameDuration = duration
      device.activeVideoMaxFrameDuration = duration
    }
    device.videoZoomFactor = 1
    device.unlockForConfiguration()
    return choice
  }

  func unbind() {
    queue.async {
      if self.movieOutput?.isRecording == true { self.movieOutput?.stopRecording() }
      self.session.stopRunning()
      self.session.beginConfiguration()
      self.session.inputs.forEach(self.session.removeInput)
      self.session.outputs.forEach(self.session.removeOutput)
      self.session.commitConfiguration()
      self.camera = nil
      self.photoOutput = nil
      self.movieOutput = nil
      self.frameOutput = nil
      self.rotation = nil
      self.recording = nil
    }
  }

  func takePhoto(flash: Bool, profile: CaptureProfile) async throws -> Data {
    try await withCheckedThrowingContinuation { continuation in
      queue.async {
        guard let output = self.photoOutput, self.session.isRunning else {
          return continuation.resume(throwing: MediaError.Camera(reason: .notReady, detail: cameraRefusalText(reason: .notReady)))
        }
        let settings = AVCapturePhotoSettings(format: [
          AVVideoCodecKey: AVVideoCodecType.jpeg,
          AVVideoCompressionPropertiesKey: [AVVideoQualityKey: Double(profile.photoQuality) / 100]
        ])
        settings.maxPhotoDimensions = output.maxPhotoDimensions
        settings.flashMode = flash && output.supportedFlashModes.contains(.on) ? .on : .off
        if profile.reducesRedEye, output.isAutoRedEyeReductionSupported { settings.isAutoRedEyeReductionEnabled = true }
        let id = UUID()
        let delegate = PhotoDelegate { result in
          self.queue.async { self.photoDelegates[id] = nil }
          continuation.resume(with: result)
        }
        self.photoDelegates[id] = delegate
        output.capturePhoto(with: settings, delegate: delegate)
      }
    }
  }

  func beginRecording(to url: URL, profile: CaptureProfile, lens: CameraLens) {
    queue.async { [weak self] in
      guard let self else { return }
      guard let output = self.movieOutput, !output.isRecording, self.session.isRunning else { return self.emit(.recordFinished(end: .failed)) }
      if let connection = output.connection(with: .video) {
        if connection.isVideoMirroringSupported {
          connection.automaticallyAdjustsVideoMirroring = false
          connection.isVideoMirrored = lens == .front && profile.mirrorFrontVideo
        }
        let angle = self.rotation?.videoRotationAngleForHorizonLevelCapture ?? 90
        if connection.isVideoRotationAngleSupported(angle) { connection.videoRotationAngle = angle }
        if let camera = self.camera {
          let supported = [VideoStabilization.standard, .cinematic].filter { camera.activeFormat.isVideoStabilizationModeSupported($0.avMode) }
          if let mode = stabilizationMode(requested: profile.stabilization, supported: supported) { connection.preferredVideoStabilizationMode = mode.avMode }
        }
      }
      output.maxRecordedDuration = CMTime(value: CMTimeValue(profile.maxDurationMs), timescale: 1000)
      let delegate = MovieDelegate(onStart: { [weak self] in self?.emit(.recordStarted) }, onFinish: { [weak self] end in self?.emit(.recordFinished(end: end)) })
      self.recording = delegate
      output.startRecording(to: url, recordingDelegate: delegate)
    }
  }

  func endRecording() {
    queue.async { if self.movieOutput?.isRecording == true { self.movieOutput?.stopRecording() } }
  }

  func recordedMs() -> UInt64 {
    let seconds = movieOutput?.recordedDuration.seconds ?? 0
    return seconds.isFinite && seconds > 0 ? UInt64(seconds * 1000) : 0
  }

  func setTorch(_ on: Bool) {
    queue.async {
      guard let camera = self.camera, camera.hasTorch, (try? camera.lockForConfiguration()) != nil else { return }
      camera.torchMode = on ? .on : .off
      camera.unlockForConfiguration()
    }
  }

  func setZoom(_ ratio: Float) {
    queue.async {
      guard let camera = self.camera, (try? camera.lockForConfiguration()) != nil else { return }
      camera.videoZoomFactor = CGFloat(ratio)
      camera.unlockForConfiguration()
    }
  }

  func setFrameTap(_ tap: (@Sendable (CVPixelBuffer, UInt64) -> Void)?) {
    lock.withLock { self.tap = tap }
    queue.async { self.installFrameOutput() }
  }

  private func installFrameOutput() {
    if lock.withLock({ tap }) == nil {
      guard let output = frameOutput else { return }
      session.beginConfiguration()
      session.removeOutput(output)
      session.commitConfiguration()
      frameOutput = nil
      return
    }
    guard frameOutput == nil, camera != nil else { return }
    let output = AVCaptureVideoDataOutput()
    output.alwaysDiscardsLateVideoFrames = true
    output.setSampleBufferDelegate(self, queue: frames)
    session.beginConfiguration()
    if session.canAddOutput(output) {
      session.addOutput(output)
      frameOutput = output
    }
    session.commitConfiguration()
  }
}

extension AVCaptureBackend: AVCaptureVideoDataOutputSampleBufferDelegate {
  func captureOutput(_ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection) {
    guard let handler = lock.withLock({ tap }), let buffer = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
    let seconds = CMSampleBufferGetPresentationTimeStamp(sampleBuffer).seconds
    handler(buffer, seconds.isFinite && seconds > 0 ? UInt64(seconds * 1000) : 0)
  }
}

private final class PhotoDelegate: NSObject, AVCapturePhotoCaptureDelegate, @unchecked Sendable {
  private let done: @Sendable (Result<Data, Error>) -> Void

  init(done: @escaping @Sendable (Result<Data, Error>) -> Void) {
    self.done = done
  }

  func photoOutput(_ output: AVCapturePhotoOutput, didFinishProcessingPhoto photo: AVCapturePhoto, error: Error?) {
    if let error { return done(.failure(error)) }
    guard let data = photo.fileDataRepresentation() else {
      return done(.failure(MediaError.Camera(reason: .photoFailed, detail: cameraRefusalText(reason: .photoFailed))))
    }
    done(.success(data))
  }
}

private final class MovieDelegate: NSObject, AVCaptureFileOutputRecordingDelegate, @unchecked Sendable {
  private let onStart: @Sendable () -> Void
  private let onFinish: @Sendable (RecordEnd) -> Void

  init(onStart: @escaping @Sendable () -> Void, onFinish: @escaping @Sendable (RecordEnd) -> Void) {
    self.onStart = onStart
    self.onFinish = onFinish
  }

  func fileOutput(_ output: AVCaptureFileOutput, didStartRecordingTo fileURL: URL, from connections: [AVCaptureConnection]) {
    onStart()
  }

  func fileOutput(_ output: AVCaptureFileOutput, didFinishRecordingTo outputFileURL: URL, from connections: [AVCaptureConnection], error: Error?) {
    onFinish(Self.end(of: error))
  }

  static func end(of error: Error?) -> RecordEnd {
    guard let error else { return .completed }
    let failure = error as NSError
    if failure.code == AVError.maximumDurationReached.rawValue { return .limitReached }
    if failure.userInfo[AVErrorRecordingSuccessfullyFinishedKey] as? Bool == true { return .completed }
    return .failed
  }
}

private extension VideoStabilization {
  var avMode: AVCaptureVideoStabilizationMode {
    switch self {
    case .off: .off
    case .standard: .standard
    case .cinematic: .cinematic
    }
  }
}
#endif
