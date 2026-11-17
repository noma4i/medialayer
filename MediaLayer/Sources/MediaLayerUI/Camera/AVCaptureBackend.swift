#if os(iOS)
@preconcurrency import AVFoundation
import CoreMedia
import Foundation
import MediaLayer

/// The hardware camera: one `AVCaptureSession` with the photo output of the profile. A video take is written by `CameraTake` from the audio and video data outputs. Every call runs on its own serial queue, never on Main.
final class AVCaptureBackend: NSObject, CaptureBackend, CameraMicrophone, @unchecked Sendable {
  let session = AVCaptureSession()
  private let queue = DispatchQueue(label: "com.noma4i.medialayer.camera.session")
  private let frames = DispatchQueue(label: "com.noma4i.medialayer.camera.frames")
  private let lock = NSLock()
  private var sink: (@Sendable (CaptureEvent) -> Void)?
  private var camera: AVCaptureDevice?
  private var photoOutput: AVCapturePhotoOutput?
  private var videoMode = false
  private var audioOutput: AVCaptureAudioDataOutput?
  private var microphoneInput: AVCaptureDeviceInput?
  private var audioProfile = false
  private var wanted: (generation: UInt64, listener: CameraMicrophoneListener)?
  private var audioFlowing = false
  private var audioInterrupted = false
  private var frameOutput: AVCaptureVideoDataOutput?
  private var rotation: AVCaptureDevice.RotationCoordinator?
  private var relay: CameraTakeRelay!
  private var recordingOutput = false
  private var stabilization: AVCaptureVideoStabilizationMode?
  private var photoDelegates: [UUID: PhotoDelegate] = [:]
  private var tap: (@Sendable (CVPixelBuffer, UInt64) -> Void)?
  private var chosenPhotoSize: PixelSize?
  private var tokens: [NSObjectProtocol] = []
  private var previews: [ObjectIdentifier: WeakPreview] = [:]
  private var previewLinks: [ObjectIdentifier: AVCaptureConnection] = [:]
  private var videoPort: AVCaptureInput.Port?

  override init() {
    super.init()
    relay = CameraTakeRelay(
      queue: queue, emit: { [weak self] in self?.emit($0) },
      idle: { [weak self] in
        self?.recordingOutput = false
        self?.installFrameOutput()
      }
    )
    session.automaticallyConfiguresApplicationAudioSession = false
    let center = NotificationCenter.default
    tokens = [
      center.addObserver(forName: AVCaptureSession.didStartRunningNotification, object: session, queue: nil) { [weak self] _ in self?.emit(.streaming(on: true)) },
      center.addObserver(forName: AVCaptureSession.didStopRunningNotification, object: session, queue: nil) { [weak self] _ in self?.emit(.streaming(on: false)) },
      center.addObserver(forName: AVCaptureSession.wasInterruptedNotification, object: session, queue: nil) { [weak self] note in
        let reason = (note.userInfo?[AVCaptureSessionInterruptionReasonKey] as? NSNumber)?.intValue
        self?.queue.async { [weak self] in
          guard let self else { return }
          MediaTrace.emit("camera", "interruption reason=\(reason.map(String.init) ?? "unknown")")
          if reason == AVCaptureSession.InterruptionReason.audioDeviceInUseByAnotherClient.rawValue, audioOutput != nil {
            audioInterrupted = true
            audioFlowing = false
            emit(.microphoneFlow(false))
            loseMicrophone(nil)
          }
          emit(.streaming(on: false))
        }
      },
      center.addObserver(forName: AVCaptureSession.interruptionEndedNotification, object: session, queue: nil) { [weak self] _ in
        guard let self else { return }
        self.queue.async {
          self.audioInterrupted = false
          self.emit(.streaming(on: self.session.isRunning))
        }
      },
      center.addObserver(forName: AVCaptureSession.runtimeErrorNotification, object: session, queue: nil) { [weak self] note in
        let error = note.userInfo?[AVCaptureSessionErrorKey] as? AVError
        self?.emit(.fault(kind: error?.code == .mediaServicesWereReset ? .reset : .other))
        self?.queue.async { if self?.audioOutput != nil { self?.loseMicrophone(error ?? NSError(domain: AVFoundationErrorDomain, code: AVError.unknown.rawValue)) } }
      }
    ]
  }

  deinit {
    tokens.forEach(NotificationCenter.default.removeObserver)
    session.stopRunning()
  }

  var photoSize: PixelSize? { lock.withLock { chosenPhotoSize } }
  var audio: CaptureAudio { .native(self) }

  func setSink(_ sink: @escaping @Sendable (CaptureEvent) -> Void) {
    lock.withLock { self.sink = sink }
  }

  private func emit(_ event: CaptureEvent) {
    if case let .refuse(reason) = event { MediaTrace.emit("camera", "refuse reason=\(reason)") }
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
      inputs = [try AVCaptureDeviceInput(device: device)]
    } catch {
      Media.recordNativeFailure(error, source: .camera)
      return emit(.refuse(reason: .busy))
    }
    let photo = profile.mode == .photo ? AVCapturePhotoOutput() : nil
    relay.cancel()
    recordingOutput = false
    session.beginConfiguration()
    session.inputs.forEach(session.removeInput)
    session.outputs.forEach(session.removeOutput)
    previewLinks = [:]
    videoPort = nil
    frameOutput = nil
    audioOutput = nil
    microphoneInput = nil
    audioProfile = profile.audio
    var outputs: [AVCaptureOutput] = []
    if let photo { outputs.append(photo) }
    let added = inputs.allSatisfy { session.canAddInput($0) } && outputs.allSatisfy { session.canAddOutput($0) }
    guard added else {
      session.commitConfiguration()
      return emit(.refuse(reason: .configureFailed))
    }
    inputs.forEach(session.addInput)
    outputs.forEach(session.addOutput)
    let microphoneFailure = addMicrophone()
    let choice: FormatChoice?
    do { choice = try applyFormat(device, profile: profile) }
    catch {
      Media.recordNativeFailure(error, source: .camera)
      choice = nil
    }
    let supported = [VideoStabilization.standard, .cinematic].filter { device.activeFormat.isVideoStabilizationModeSupported($0.avMode) }
    stabilization = profile.mode == .video ? stabilizationMode(requested: profile.stabilization, supported: supported)?.avMode : nil
    videoPort = inputs.first?.ports(for: .video, sourceDeviceType: device.deviceType, sourceDevicePosition: device.position).first
    previews.keys.forEach(linkPreview)
    session.commitConfiguration()
    guard session.hardwareCost <= 1 else {
      session.beginConfiguration()
      session.inputs.forEach(session.removeInput)
      session.outputs.forEach(session.removeOutput)
      session.commitConfiguration()
      audioOutput = nil
      microphoneInput = nil
      return emit(.refuse(reason: .configureFailed))
    }
    camera = device
    photoOutput = photo
    videoMode = profile.mode == .video
    audioFlowing = false
    audioInterrupted = false
    if audioOutput != nil { emit(.microphoneFlow(false)) }
    if let microphoneFailure { loseMicrophone(microphoneFailure) }
    rotation = AVCaptureDevice.RotationCoordinator(device: device, previewLayer: nil)
    if let photo, let size = choice?.photoSize {
      photo.maxPhotoDimensions = CMVideoDimensions(width: Int32(size.width), height: Int32(size.height))
    }
    lock.withLock { chosenPhotoSize = choice?.photoSize }
    installFrameOutput()
    if !session.isRunning { session.startRunning() }
    MediaTrace.emit("camera", "bind audio=\(profile.audio) microphone=\(session.inputs.contains { $0.ports.contains { $0.mediaType == .audio } }) videoMode=\(profile.mode == .video)")
    let hasFlash = profile.mode == .photo ? device.isFlashAvailable : device.hasTorch
    emit(.opened(hasFlash: hasFlash, zoomMin: Float(device.minAvailableVideoZoomFactor), zoomMax: Float(device.maxAvailableVideoZoomFactor)))
    emit(.streaming(on: session.isRunning))
  }

  private func applyFormat(_ device: AVCaptureDevice, profile: CaptureProfile) throws -> FormatChoice? {
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
    guard let choice = pickFormat(candidates: candidates, profile: profile) else { return nil }
    try device.lockForConfiguration()
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

  /// The preview layer joins the session here, on the session queue: both the join and its connection take the session lock, so Main never touches the session.
  func attachPreviewLayer(_ layer: AVCaptureVideoPreviewLayer) {
    nonisolated(unsafe) let layer = layer
    queue.async {
      let id = ObjectIdentifier(layer)
      layer.setSessionWithNoConnection(self.session)
      self.previews[id] = WeakPreview(layer)
      guard self.videoPort != nil else { return }
      self.session.beginConfiguration()
      self.linkPreview(id)
      self.session.commitConfiguration()
    }
  }

  func detachPreviewLayer(_ layer: AVCaptureVideoPreviewLayer) {
    nonisolated(unsafe) let layer = layer
    queue.async {
      let id = ObjectIdentifier(layer)
      self.previews[id] = nil
      guard let link = self.previewLinks.removeValue(forKey: id) else { return }
      self.session.beginConfiguration()
      self.session.removeConnection(link)
      self.session.commitConfiguration()
    }
  }

  private func linkPreview(_ id: ObjectIdentifier) {
    guard previewLinks[id] == nil, let port = videoPort else { return }
    guard let layer = previews[id]?.layer else {
      previews[id] = nil
      return
    }
    let link = AVCaptureConnection(inputPort: port, videoPreviewLayer: layer)
    guard session.canAddConnection(link) else { return }
    session.addConnection(link)
    previewLinks[id] = link
  }

  func unbind() {
    unbind(completion: {})
  }

  func unbind(completion: @escaping @Sendable () -> Void) {
    queue.async {
      self.relay.cancel()
      self.recordingOutput = false
      self.session.stopRunning()
      self.session.beginConfiguration()
      self.session.inputs.forEach(self.session.removeInput)
      self.session.outputs.forEach(self.session.removeOutput)
      self.session.commitConfiguration()
      self.previewLinks = [:]
      self.videoPort = nil
      self.camera = nil
      self.photoOutput = nil
      self.videoMode = false
      self.stabilization = nil
      self.audioOutput = nil
      self.microphoneInput = nil
      self.audioFlowing = false
      self.audioInterrupted = false
      self.emit(.microphoneFlow(true))
      self.frameOutput = nil
      self.rotation = nil
      completion()
    }
  }

  func takePhoto(flash: Bool, profile: CaptureProfile) async throws -> Data {
    try await withCheckedThrowingContinuation { continuation in
      queue.async {
        guard let output = self.photoOutput, self.session.isRunning else {
          return continuation.resume(throwing: MediaError.Camera(reason: .notReady, detail: cameraRefusalText(reason: .notReady), recordedMs: nil))
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
    beginRecording(to: url, profile: profile, lens: lens, ticket: nil, refused: { _ in })
  }

  func beginRecording(to url: URL, profile: CaptureProfile, lens: CameraLens, ticket: RecordingTicket?, refused: @escaping @Sendable (RecordingRefusal) -> Void) {
    queue.async { [weak self] in
      guard let self else { return }
      guard videoMode, camera != nil, !relay.isActive, session.isRunning else { return emit(.recordFinished(end: .failed)) }
      recordingOutput = true
      installFrameOutput()
      guard let settings = takeSettings(profile: profile, lens: lens) else {
        Media.recordNativeFailure(NSError(domain: AVFoundationErrorDomain, code: -1), source: .camera)
        recordingOutput = false
        installFrameOutput()
        return emit(.recordFinished(end: .failed))
      }
      relay.begin(url: url, settings: settings, ticket: ticket, refused: refused)
    }
  }

  private func takeSettings(profile: CaptureProfile, lens: CameraLens) -> CameraTake.Settings? {
    let fileType = AVFileType.mp4
    guard let video = frameOutput?.recommendedVideoSettingsForAssetWriter(writingTo: fileType), let clock = session.synchronizationClock else { return nil }
    var audio: [String: Any]?
    if profile.audio {
      guard let settings = audioOutput?.recommendedAudioSettingsForAssetWriter(writingTo: fileType) as? [String: Any] else { return nil }
      audio = settings
    }
    var transform = CGAffineTransform(rotationAngle: CGFloat(rotation?.videoRotationAngleForHorizonLevelCapture ?? 90) * .pi / 180)
    if lens == .front && profile.mirrorFrontVideo { transform = transform.concatenating(CGAffineTransform(scaleX: -1, y: 1)) }
    let budget = Int(clamping: captureCost(profile: profile))
    return CameraTake.Settings(
      video: video, audio: audio, transform: transform, maxDurationMs: profile.maxDurationMs, fileType: fileType,
      fragmentInterval: Self.fragmentInterval, armPTS: CMClockGetTime(clock), maxPendingAudio: budget / CameraTake.entryCost, maxPendingVideo: budget / CameraTake.entryCost,
      maxHeldAudio: budget / CameraTake.entryCost, memoryBudgetBytes: budget
    )
  }

  private static let fragmentInterval = CMTime(seconds: 10, preferredTimescale: 600)

  func endRecording() {
    queue.async { self.relay.stop() }
  }

  func recordedMs() -> UInt64 {
    relay.recordedMs
  }

  func start(generation: UInt64, listener: CameraMicrophoneListener) {
    queue.async {
      self.removeMicrophone()
      self.wanted = (generation, listener)
      guard self.camera != nil else { return }
      self.session.beginConfiguration()
      let failure = self.addMicrophone()
      self.session.commitConfiguration()
      if self.audioOutput != nil {
        self.audioFlowing = false
        self.emit(.microphoneFlow(false))
      }
      if let failure { self.loseMicrophone(failure) }
    }
  }

  func stop(generation: UInt64) {
    queue.async {
      guard self.wanted?.generation == generation else { return }
      self.wanted = nil
      self.removeMicrophone()
    }
  }

  private func addMicrophone() -> Error? {
    guard audioProfile, wanted != nil, audioOutput == nil else { return nil }
    do {
      guard authorized(.audio) else { throw NSError(domain: AVFoundationErrorDomain, code: AVError.applicationIsNotAuthorizedToUseDevice.rawValue) }
      guard let device = AVCaptureDevice.default(for: .audio) else { throw NSError(domain: AVFoundationErrorDomain, code: AVError.deviceNotConnected.rawValue) }
      let input = try AVCaptureDeviceInput(device: device)
      let output = AVCaptureAudioDataOutput()
      guard session.canAddInput(input), session.canAddOutput(output) else { throw NSError(domain: AVFoundationErrorDomain, code: AVError.sessionConfigurationChanged.rawValue) }
      output.setSampleBufferDelegate(self, queue: queue)
      session.addInput(input)
      session.addOutput(output)
      microphoneInput = input
      audioOutput = output
      return nil
    } catch {
      return error
    }
  }

  private func removeMicrophone() {
    guard let input = microphoneInput, let output = audioOutput else { return }
    session.beginConfiguration()
    session.removeOutput(output)
    session.removeInput(input)
    session.commitConfiguration()
    microphoneInput = nil
    audioOutput = nil
    audioFlowing = false
    emit(.microphoneFlow(false))
  }

  private func loseMicrophone(_ error: Error?) {
    guard let wanted else { return }
    wanted.listener.microphoneLost(generation: wanted.generation, error: error)
  }

  func setTorch(_ on: Bool) throws {
    try queue.sync {
      guard let camera = self.camera, camera.hasTorch else { throw NSError(domain: AVFoundationErrorDomain, code: -1) }
      try camera.lockForConfiguration()
      camera.torchMode = on ? .on : .off
      camera.unlockForConfiguration()
    }
  }

  func setZoom(_ ratio: Float) throws {
    try queue.sync {
      guard let camera = self.camera else { throw NSError(domain: AVFoundationErrorDomain, code: -1) }
      try camera.lockForConfiguration()
      camera.videoZoomFactor = CGFloat(ratio)
      camera.unlockForConfiguration()
    }
  }

  func setFrameTap(_ tap: (@Sendable (CVPixelBuffer, UInt64) -> Void)?) {
    lock.withLock { self.tap = tap }
    queue.async { self.installFrameOutput() }
  }

  private func installFrameOutput() {
    if lock.withLock({ tap }) == nil && !recordingOutput {
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
      if let stabilization { output.connection(with: .video)?.preferredVideoStabilizationMode = stabilization }
    }
    session.commitConfiguration()
  }
}

extension AVCaptureBackend: AVCaptureVideoDataOutputSampleBufferDelegate, AVCaptureAudioDataOutputSampleBufferDelegate {
  func captureOutput(_ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection) {
    if output is AVCaptureAudioDataOutput {
      guard output === audioOutput, !audioInterrupted, isLivePCM(sampleBuffer) else { return }
      relay.appendAudio(sampleBuffer)
      if let wanted { wanted.listener.microphoneHeard(generation: wanted.generation) }
      guard !audioFlowing else { return }
      audioFlowing = true
      MediaTrace.emit("camera", "first audio sample")
      emit(.microphoneFlow(true))
      return
    }
    relay.appendVideo(sampleBuffer)
    guard let handler = lock.withLock({ tap }), let buffer = CMSampleBufferGetImageBuffer(sampleBuffer) else { return }
    let seconds = CMSampleBufferGetPresentationTimeStamp(sampleBuffer).seconds
    handler(buffer, seconds.isFinite && seconds > 0 ? UInt64(seconds * 1000) : 0)
  }
}

private struct WeakPreview {
  weak var layer: AVCaptureVideoPreviewLayer?

  init(_ layer: AVCaptureVideoPreviewLayer) {
    self.layer = layer
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
      return done(.failure(MediaError.Camera(reason: .photoFailed, detail: cameraRefusalText(reason: .photoFailed), recordedMs: nil)))
    }
    done(.success(data))
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
