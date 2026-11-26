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
  private var lensRotation: AVCaptureDevice.RotationCoordinator?
  private var poseRotation: AVCaptureDevice.RotationCoordinator?
  private var portraitAngle: CGFloat?
  private var movieDefaultAngle: CGFloat = 0
  private var frameOutput: AVCaptureVideoDataOutput?
  private var recording: MovieDelegate?
  private var photoDelegates: [UUID: PhotoDelegate] = [:]
  private var tap: (@Sendable (CVPixelBuffer, UInt64) -> Void)?
  private var chosenPhotoSize: PixelSize?
  private var tokens: [NSObjectProtocol] = []
  private var previews: [ObjectIdentifier: WeakPreview] = [:]
  private var previewLinks: [ObjectIdentifier: AVCaptureConnection] = [:]
  private var videoPort: AVCaptureInput.Port?
  /// Same rule as the movie connection (front lens and profile.mirrorFrontVideo, or mirrorFrontPhoto for a photo profile), so the preview shows the picture of the file.
  private var previewMirrored = false
  private var previewLens: CameraLens = .back
  private var previewMirrorFrontVideo = false

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
      Media.recordNativeFailure(error, source: .camera)
      return emit(.refuse(reason: .busy))
    }
    let photo = profile.mode == .photo ? AVCapturePhotoOutput() : nil
    let movie = profile.mode == .video ? AVCaptureMovieFileOutput() : nil
    // A fragmented movie (default every 10 s) hides its audio track from AVFoundation: write one plain movie.
    movie?.movieFragmentInterval = .invalid
    session.beginConfiguration()
    session.inputs.forEach(session.removeInput)
    session.outputs.forEach(session.removeOutput)
    previewLinks = [:]
    videoPort = nil
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
    videoPort = inputs.first?.ports(for: .video, sourceDeviceType: device.deviceType, sourceDevicePosition: device.position).first
    let mirrorFront = profile.mode == .photo ? profile.mirrorFrontPhoto : profile.mirrorFrontVideo
    previewMirrored = lens == .front && mirrorFront
    previewLens = lens
    previewMirrorFrontVideo = mirrorFront
    previews.keys.forEach(linkPreview)
    // The movie file never carries a mirror: the stop path mirrors the pixels. The connection is set once per bind: a take only starts writing, the running session is not touched at its start.
    if let connection = movie?.connection(with: .video), connection.isVideoMirroringSupported {
      connection.automaticallyAdjustsVideoMirroring = false
      connection.isVideoMirrored = false
    }
    session.commitConfiguration()
    camera = device
    photoOutput = photo
    movieOutput = movie
    let lensCoordinator = AVCaptureDevice.RotationCoordinator(device: device, previewLayer: nil)
    lensRotation = lensCoordinator
    poseRotation = lens == .back ? lensCoordinator : AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .back).map { AVCaptureDevice.RotationCoordinator(device: $0, previewLayer: nil) }
    portraitAngle = nil
    movieDefaultAngle = movie?.connection(with: .video)?.videoRotationAngle ?? 0
    let choice: FormatChoice?
    do { choice = try applyFormat(device, profile: profile) }
    catch {
      Media.recordNativeFailure(error, source: .camera)
      choice = nil
    }
    // Stabilization depends on the active format, so it is set after applyFormat, still inside the bind.
    if let connection = movie?.connection(with: .video) {
      let supported = [VideoStabilization.standard, .cinematic].filter { device.activeFormat.isVideoStabilizationModeSupported($0.avMode) }
      if let mode = stabilizationMode(requested: profile.stabilization, supported: supported) {
        session.beginConfiguration()
        connection.preferredVideoStabilizationMode = mode.avMode
        session.commitConfiguration()
      }
    }
    if let photo, let size = choice?.photoSize {
      photo.maxPhotoDimensions = CMVideoDimensions(width: Int32(size.width), height: Int32(size.height))
    }
    lock.withLock { chosenPhotoSize = choice?.photoSize }
    installFrameOutput()
    if !session.isRunning { session.startRunning() }
    _ = takeAngle()
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
    if link.isVideoMirroringSupported {
      link.automaticallyAdjustsVideoMirroring = false
      link.isVideoMirrored = previewMirrored
    }
    MediaTrace.emit("camera", "preview lens=\(previewLens == .front ? "FRONT" : "BACK") mirrorFront=\(previewMirrorFrontVideo) previewMirrored=\(link.isVideoMirrored)")
    previewLinks[id] = link
  }

  /// The take angle of the lens is Apple's capture angle of that lens while the phone is upright, so a take keeps the portrait frame in any pose.
  private func takeAngle() -> CGFloat {
    let capture = lensRotation?.videoRotationAngleForHorizonLevelCapture
    let upright = poseRotation.map { $0.videoRotationAngleForHorizonLevelCapture == 90 } ?? true
    if upright, let capture { portraitAngle = capture }
    let angle = portraitAngle ?? capture.flatMap { upright ? $0 : nil } ?? (movieDefaultAngle + 90).truncatingRemainder(dividingBy: 360)
    MediaTrace.emit("camera", "take angle=\(angle) capture=\(capture.map { "\($0)" } ?? "none") pose=\(poseRotation.map { "\($0.videoRotationAngleForHorizonLevelCapture)" } ?? "none") default=\(movieDefaultAngle) learned=\(portraitAngle != nil)")
    return angle
  }

  func unbind() {
    unbind(completion: {})
  }

  func unbind(completion: @escaping @Sendable () -> Void) {
    queue.async {
      if self.movieOutput?.isRecording == true { self.movieOutput?.stopRecording() }
      self.session.stopRunning()
      self.session.beginConfiguration()
      self.session.inputs.forEach(self.session.removeInput)
      self.session.outputs.forEach(self.session.removeOutput)
      self.session.commitConfiguration()
      self.previewLinks = [:]
      self.videoPort = nil
      self.camera = nil
      self.photoOutput = nil
      self.movieOutput = nil
      self.lensRotation = nil
      self.poseRotation = nil
      self.portraitAngle = nil
      self.frameOutput = nil
      self.recording = nil
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
    queue.async { [weak self] in
      guard let self else { return }
      guard let output = self.movieOutput, !output.isRecording, self.session.isRunning else { return self.emit(.recordFinished(end: .failed)) }
      if let connection = output.connection(with: .video) {
        let angle = self.takeAngle()
        if connection.isVideoRotationAngleSupported(angle) { connection.videoRotationAngle = angle }
      }
      // The movie output writes the sound itself: the core only needs to hear that the microphone of this take is live.
      if profile.audio { self.emit(.audioFlow(flowing: true)) }
      output.maxRecordedDuration = CMTime(value: CMTimeValue(profile.maxDurationMs), timescale: 1000)
      let delegate = MovieDelegate(onStart: { [weak self] in
        guard let self else { return }
        if let connection = self.movieOutput?.connection(with: .video) {
          NSLog("movie stabilization preferred=%ld active=%ld", connection.preferredVideoStabilizationMode.rawValue, connection.activeVideoStabilizationMode.rawValue)
        }
        self.emit(.recordStarted)
      }, onFinish: { [weak self] end in self?.emit(.recordFinished(end: end)) })
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
