#if os(iOS)
import AVFoundation
import Foundation
import ImageIO
import MediaLayer

/// A photo from the camera: an upright JPEG file (the EXIF turn is applied, the mirror is not), its size after the turn and the side it was taken from.
public struct CapturedPhoto: Sendable, Equatable {
  public let url: URL
  public let width: Int
  public let height: Int
  public let lens: CameraLens
}

/// A recorded clip: an mp4 file, its length and its frame as the screen shows it. The file belongs to the app from now on.
public struct CapturedClip: Sendable, Equatable {
  public let url: URL
  public let durationMs: UInt64
  public let frame: ImageSize?
  public let lens: CameraLens
}

extension CaptureProfile {
  public static var photo: CaptureProfile { captureProfile(kind: .photo) }
  public static var moment: CaptureProfile { captureProfile(kind: .moment) }
  public static var circle: CaptureProfile { captureProfile(kind: .circle) }
}

extension CameraState {
  public var isReady: Bool { phase == .ready }
  public var isRecording: Bool { phase == .recording }
}

/// The place of a camera: it takes photos and records video with the profile it was made with. The session runs only while a `MediaCameraPreview` of this camera is on screen
/// and the app is in front, so the app never prepares, resumes or closes the camera. A command that does not fit the state is ignored and the state shows the result.
/// A refused permission, a busy camera, a recording that is too short and a refusal of the memory budget are a `MediaError.Camera` with the reason and the text of the core.
public final class MediaCamera: @unchecked Sendable {
  private let profile: CaptureProfile
  private let services: CameraServices
  private let backend: CaptureBackend
  private let queue = DispatchQueue(label: "com.noma4i.medialayer.camera")
  private let lock = NSLock()
  private var published: CameraState
  private var sinks: [UUID: AsyncStream<CameraState>.Continuation] = [:]

  private var run: CaptureRun
  private var demand = 0
  private var foreground = true
  private var engaged = false
  private var epoch = 0
  private var sessionHold: CameraServices.Release?
  private var audioHold: RecordingTicket?
  private var microphoneFlowing = true
  private var binding: Task<Void, Never>?
  private var shooting: Task<Void, Never>?
  private var reading: Task<Void, Never>?
  private var ticker: Task<Void, Never>?
  private var recordingURL: URL?
  private var clipInfo: MediaInfo?
  private var ended: Result<CapturedClip, Error>?
  private var photoWaiter: CheckedContinuation<CapturedPhoto, Error>?
  private var startWaiter: CheckedContinuation<Void, Error>?
  private var stopWaiter: CheckedContinuation<CapturedClip, Error>?
  private var signalTokens: [NSObjectProtocol] = []
  private var observers: [UUID: (rate: Double, last: UInt64?, handler: @Sendable (CameraFrame) -> Void)] = [:]

  public convenience init(profile: CaptureProfile, lens: CameraLens = .back) throws {
    try self.init(profile: profile, lens: lens, backend: AVCaptureBackend(), services: .live)
  }

  init(profile: CaptureProfile, lens: CameraLens, backend: CaptureBackend, services: CameraServices) throws {
    self.profile = try checkCaptureProfile(profile: profile)
    self.services = services
    self.backend = backend
    run = captureRun(lens: lens)
    published = run.state
    backend.setSink { [weak self] event in self?.post(event) }
    signalTokens = [
      services.center.addObserver(forName: services.signals.background, object: nil, queue: nil) { [weak self] _ in self?.setForeground(false) },
      services.center.addObserver(forName: services.signals.foreground, object: nil, queue: nil) { [weak self] _ in self?.setForeground(true) }
    ]
  }

  deinit {
    signalTokens.forEach(services.center.removeObserver)
    let backend = backend
    backend.setSink { _ in }
    backend.setFrameTap(nil)
    backend.unbind()
    for task in [binding, shooting, reading, ticker] { task?.cancel() }
    sessionHold?()
    audioHold?.end()
    if let url = recordingURL { try? FileManager.default.removeItem(at: url) }
  }

  public static func isAvailable(_ lens: CameraLens) -> Bool {
    AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: lens == .front ? .front : .back) != nil
  }

  /// The state now: readiness, recording and its length, flash, zoom, the side and the last refusal.
  public var state: CameraState { lock.withLock { published } }

  func tickerTask() async -> Task<Void, Never>? {
    await withCheckedContinuation { continuation in
      queue.async { continuation.resume(returning: self.ticker) }
    }
  }

  /// The states as they change. The stream gives the current state at once and ends when the observer stops listening.
  public var states: AsyncStream<CameraState> {
    let id = UUID()
    return AsyncStream(bufferingPolicy: .bufferingNewest(1)) { continuation in
      lock.withLock {
        sinks[id] = continuation
        continuation.yield(published)
      }
      continuation.onTermination = { [weak self] _ in self?.lock.withLock { self?.sinks[id] = nil } }
    }
  }

  public func takePhoto() async throws -> CapturedPhoto {
    try await withCheckedThrowingContinuation { continuation in
      queue.async {
        guard self.photoWaiter == nil else { return continuation.resume(throwing: Self.refusal(.notReady)) }
        self.apply(.photo)
        if self.run.state.shooting { self.photoWaiter = continuation } else { continuation.resume(throwing: Self.refusal(.notReady)) }
      }
    }
  }

  /// Returns after the hardware started the recording. `NotReady` when the camera is not ready or the profile holds no video.
  public func startRecording() async throws {
    if profile.audio { try await ensureAudioTicket() }
    do {
      try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, Error>) in
        queue.async {
          guard self.startWaiter == nil else { return continuation.resume(throwing: Self.refusal(.notReady)) }
          if self.profile.audio && self.audioHold == nil { return continuation.resume(throwing: Self.refusal(.notReady)) }
          self.ended = nil
          let wasReady = self.run.state.phase == .ready && self.profile.mode == .video
          self.apply(.start)
          if self.run.starting { self.startWaiter = continuation } else { continuation.resume(throwing: Self.refusal(wasReady ? self.run.state.fault ?? .notReady : .notReady)) }
        }
      }
    } catch {
      await withCheckedContinuation { continuation in
        queue.async {
          if !self.run.started && !self.run.starting {
            if let url = self.recordingURL { try? FileManager.default.removeItem(at: url) }
            self.recordingURL = nil
          }
          continuation.resume()
        }
      }
      throw error
    }
  }

  /// Ends the recording and gives the clip. A recording that is too short or failed, or was thrown away, is a `MediaError.Camera`; a recording that is too short carries its length in `recordedMs`.
  /// After the limit ended the recording, gives the outcome of that recording.
  public func stopRecording() async throws -> CapturedClip {
    try await withCheckedThrowingContinuation { continuation in
      queue.async {
        if let outcome = self.ended {
          self.ended = nil
          return continuation.resume(with: outcome)
        }
        guard self.stopWaiter == nil, self.run.started || self.run.starting else { return continuation.resume(throwing: Self.refusal(.notReady)) }
        self.stopWaiter = continuation
        self.apply(.stop)
      }
    }
  }

  /// Throws the recording away and deletes its file.
  public func cancelRecording() {
    post(.cancel)
  }

  public func setLens(_ lens: CameraLens) {
    post(.setLens(lens: lens))
  }

  public func setFlash(_ on: Bool) {
    post(.setFlash(on: on))
  }

  public func zoom(by factor: Double) {
    post(.zoomBy(factor: Float(factor)))
  }

  /// Gives the frames of the preview to `handler`, at most `maxPerSecond` a second, for the search of a face. The camera holds no frame output while nobody listens.
  public func observeFrames(maxPerSecond: Double, _ handler: @escaping @Sendable (CameraFrame) -> Void) -> CameraFrameObservation {
    let id = UUID()
    lock.withLock { observers[id] = (maxPerSecond, nil, handler) }
    queue.async { self.installTap() }
    return CameraFrameObservation { [weak self] in
      guard let self else { return }
      self.lock.withLock { self.observers[id] = nil }
      self.queue.async { self.installTap() }
    }
  }

  func attachPreview() {
    queue.async {
      self.demand += 1
      self.reconcile()
    }
  }

  func detachPreview() {
    queue.async {
      self.demand = max(0, self.demand - 1)
      self.reconcile()
    }
  }

  func registerPreview(_ layer: AVCaptureVideoPreviewLayer) {
    let backend = backend
    queue.async { backend.attachPreviewLayer(layer) }
  }

  func unregisterPreview(_ layer: AVCaptureVideoPreviewLayer) {
    let backend = backend
    queue.async { backend.detachPreviewLayer(layer) }
  }

  private static func refusal(_ reason: CameraRefusal, recordedMs: UInt64? = nil) -> MediaError {
    MediaError.Camera(reason: reason, detail: cameraRefusalText(reason: reason), recordedMs: recordedMs)
  }

  private static func microphoneReason(_ reason: RecordingRefusal) -> CameraRefusal {
    switch reason {
    case .inCall: return .inCall
    case .interrupted: return .interrupted
    case .microphoneSilent: return .microphoneSilent
    case .permissionDenied: return .permissionDenied
    case .microphoneBusy: return .microphoneBusy
    }
  }

  private func post(_ event: CaptureEvent) {
    queue.async { self.apply(event) }
  }

  private func setForeground(_ value: Bool) {
    queue.async {
      self.foreground = value
      self.reconcile()
    }
  }

  private func reconcile() {
    let wanted = demand > 0 && foreground
    guard wanted != engaged else { return }
    engaged = wanted
    apply(wanted ? .demandOn : .demandOff)
  }

  private func apply(_ event: CaptureEvent) {
    if case let .audioFlow(flowing) = event { microphoneFlowing = flowing }
    let step = captureStep(run: run, event: event, profile: profile)
    run = step.run
    for action in step.actions { perform(action) }
    if case .recordFinished = event { apply(.microphoneFlow(microphoneFlowing)) }
    if run.discard, run.end != nil, reading != nil {
      reading?.cancel()
      reading = nil
      apply(.recordSettled(durationMs: nil))
      return
    }
    publish(run.state)
    settleWaiters()
    syncTicker()
  }

  private func publish(_ state: CameraState) {
    let sinks: [AsyncStream<CameraState>.Continuation] = lock.withLock {
      guard published != state else { return [] }
      published = state
      return Array(self.sinks.values)
    }
    for sink in sinks { sink.yield(state) }
  }

  private func settleWaiters() {
    if let waiter = startWaiter, !run.starting {
      startWaiter = nil
      if run.started { waiter.resume() } else { waiter.resume(throwing: Self.refusal(run.state.fault ?? .notReady)) }
    }
    if let waiter = photoWaiter, !run.state.shooting {
      photoWaiter = nil
      waiter.resume(with: photoOutcome ?? .failure(Self.refusal(run.state.fault ?? .notReady)))
      photoOutcome = nil
    }
    if let waiter = stopWaiter, let outcome = ended {
      stopWaiter = nil
      ended = nil
      waiter.resume(with: outcome)
    }
  }

  private var photoOutcome: Result<CapturedPhoto, Error>?

  private func syncTicker() {
    guard run.state.phase == .recording else {
      ticker?.cancel()
      ticker = nil
      return
    }
    guard ticker == nil else { return }
    let step = captureTickMs()
    ticker = Task { [weak self] in
      while !Task.isCancelled {
        try? await Task.sleep(for: .milliseconds(step))
        guard !Task.isCancelled, let self else { return }
        self.post(.tick(recordedMs: self.backend.recordedMs()))
      }
    }
  }

  private func perform(_ action: CaptureAction) {
    switch action {
    case let .bind(lens):
      open(lens)
    case let .restart(lens):
      teardownHardware()
      open(lens)
    case .unbind:
      epoch += 1
      binding?.cancel()
      shooting?.cancel()
      teardownHardware()
      releaseHolds()
      failWaiters()
    case .takePhoto:
      shoot()
    case .beginRecording:
      let url = CaptureFiles.name(.clip, in: services.directory)
      recordingURL = url
      clipInfo = nil
      backend.beginRecording(to: url, profile: profile, lens: run.state.lens, ticket: audioHold) { [weak self] reason in
        self?.post(.revoked(reason: Self.microphoneReason(reason)))
        self?.post(.recordFinished(end: .failed))
      }
    case .endRecording:
      backend.endRecording()
    case .readClip:
      readClip()
    case .deliver:
      deliver()
    case .discard:
      discard()
    case let .setTorch(on):
      backend.setTorch(on)
    case let .setZoom(ratio):
      backend.setZoom(ratio)
    }
  }

  private func teardownHardware() {
    backend.unbind()
    reading?.cancel()
    reading = nil
    if let url = recordingURL { try? FileManager.default.removeItem(at: url) }
    recordingURL = nil
    clipInfo = nil
  }

  private func releaseHolds() {
    sessionHold?()
    sessionHold = nil
    if let audio = audioHold {
      audioHold = nil
      audio.end()
    }
  }

  private func audioRevoked(_ reason: RecordingRefusal, ticket: RecordingTicket) {
    queue.async {
      guard self.audioHold === ticket else { return }
      self.audioHold = nil
      self.apply(.revoked(reason: Self.microphoneReason(reason)))
    }
  }

  private func watch(_ ticket: RecordingTicket) {
    Task { [weak self] in
      for await reason in ticket.revoked { self?.audioRevoked(reason, ticket: ticket) }
    }
  }

  private func ensureAudioTicket() async throws {
    let needed: Bool = try await withCheckedThrowingContinuation { continuation in
      queue.async {
        guard !self.run.starting, !self.run.started else { continuation.resume(throwing: Self.refusal(.notReady)); return }
        if let ticket = self.audioHold, !ticket.isActive { self.audioHold = nil }
        continuation.resume(returning: self.audioHold == nil)
      }
    }
    guard needed else { return }
    guard let link = services.link() else { throw Self.refusal(.notReady) }
    let ticket: RecordingTicket
    switch await link.admit(.video) {
    case let .success(value): ticket = value
    case let .failure(error):
      if let reason = error as? RecordingRefusal { throw Self.refusal(Self.microphoneReason(reason)) }
      throw error
    }
    let accepted = await withCheckedContinuation { continuation in
      queue.async {
        guard self.audioHold == nil, self.engaged else { continuation.resume(returning: false); return }
        self.audioHold = ticket
        self.watch(ticket)
        continuation.resume(returning: true)
      }
    }
    if !accepted { ticket.end(); throw Self.refusal(.notReady) }
  }

  private func failWaiters() {
    let failure = Self.refusal(.notReady)
    if let waiter = photoWaiter { photoWaiter = nil; waiter.resume(throwing: failure) }
    if let waiter = stopWaiter { stopWaiter = nil; waiter.resume(throwing: failure) }
  }

  private func open(_ lens: CameraLens) {
    epoch += 1
    let current = epoch
    let needsHold = sessionHold == nil
    let needsAudio = profile.audio && audioHold == nil
    binding?.cancel()
    binding = Task { [services, profile] in
      var release: CameraServices.Release?
      if needsHold {
        do {
          release = try await services.admitSession(captureCost(profile: profile))
        } catch {
          self.queue.async { if self.epoch == current { self.apply(.refuse(reason: .memory)) } }
          return
        }
      }
      var session: RecordingTicket?
      if needsAudio {
        guard let link = services.link() else {
          release?()
          self.queue.async { if self.epoch == current { self.apply(.refuse(reason: .notReady)) } }
          return
        }
        switch await link.admit(.video) {
        case let .success(ticket): session = ticket
        case let .failure(error):
          release?()
          let reason = (error as? RecordingRefusal).map(Self.microphoneReason) ?? .notReady
          self.queue.async { if self.epoch == current { self.apply(.refuse(reason: reason)) } }
          return
        }
      }
      self.queue.async {
        guard self.epoch == current else {
          release?()
          session?.end()
          return
        }
        if let release { self.sessionHold = release }
        if let session { self.audioHold = session; self.watch(session) }
        self.backend.bind(lens: lens, profile: self.profile)
        self.installTap()
      }
    }
  }

  private func shoot() {
    let current = epoch
    let flash = run.state.flash
    let lens = run.state.lens
    let sizeBytes = backend.photoSize.flatMap { shotCost(width: $0.width, height: $0.height) } ?? captureCost(profile: profile)
    shooting = Task { [services, profile, backend] in
      func finish(_ outcome: Result<CapturedPhoto, Error>) {
        self.queue.async {
          guard self.epoch == current else {
            if case let .success(photo) = outcome { try? FileManager.default.removeItem(at: photo.url) }
            return
          }
          self.photoOutcome = outcome
          self.apply(.photoDone(ok: (try? outcome.get()) != nil))
        }
      }
      let hold: CameraServices.Release
      do {
        hold = try await services.admitShot(sizeBytes)
      } catch {
        return finish(.failure(Self.refusal(.memory)))
      }
      let file = CaptureFiles.name(.photo, in: services.directory)
      let raw = file.deletingPathExtension().appendingPathExtension("raw")
      defer { try? FileManager.default.removeItem(at: raw) }
      do {
        let data = try await backend.takePhoto(flash: flash, profile: profile)
        try data.write(to: raw, options: .atomic)
        hold()
        let made = try await services.upright(raw, Double(profile.photoQuality) / 100)
        try? FileManager.default.removeItem(at: raw)
        try FileManager.default.moveItem(at: made, to: file)
        let size = Self.pixelSize(of: file)
        finish(.success(CapturedPhoto(url: file, width: size.width, height: size.height, lens: lens)))
      } catch {
        hold()
        try? FileManager.default.removeItem(at: file)
        finish(.failure(Self.refusal(.photoFailed)))
      }
    }
  }

  private static func pixelSize(of file: URL) -> (width: Int, height: Int) {
    guard let source = CGImageSourceCreateWithURL(file as CFURL, nil),
          let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
          let width = properties[kCGImagePropertyPixelWidth] as? Int,
          let height = properties[kCGImagePropertyPixelHeight] as? Int
    else { return (0, 0) }
    return (width, height)
  }

  private func readClip() {
    guard let url = recordingURL else { return }
    reading?.cancel()
    reading = Task { [services] in
      let info: MediaInfo?
      do {
        try await services.uprightVideo(url)
        try Task.checkCancellation()
        info = await services.clipInfo(url)
      } catch {
        info = nil
      }
      guard !Task.isCancelled else { return }
      self.queue.async {
        guard self.recordingURL == url else { return }
        self.reading = nil
        self.clipInfo = info
        self.apply(.recordSettled(durationMs: info?.durationMs))
      }
    }
  }

  private func deliver() {
    guard let url = recordingURL, let info = clipInfo, let length = info.durationMs else {
      discard()
      return
    }
    recordingURL = nil
    clipInfo = nil
    ended = .success(CapturedClip(url: url, durationMs: length, frame: info.frame, lens: run.state.lens))
  }

  private func discard() {
    reading?.cancel()
    reading = nil
    if let url = recordingURL { try? FileManager.default.removeItem(at: url) }
    recordingURL = nil
    clipInfo = nil
    ended = .failure(Self.refusal(run.state.fault ?? .notReady, recordedMs: run.state.refusedMs))
  }

  private func installTap() {
    guard !lock.withLock({ observers.isEmpty }) else {
      backend.setFrameTap(nil)
      return
    }
    backend.setFrameTap { [weak self] buffer, millis in self?.deliver(buffer, at: millis) }
  }

  private func deliver(_ buffer: CVPixelBuffer, at millis: UInt64) {
    let (lens, targets): (CameraLens, [@Sendable (CameraFrame) -> Void]) = lock.withLock {
      var passing: [@Sendable (CameraFrame) -> Void] = []
      for (id, entry) in observers where frameGate(lastMs: entry.last, nowMs: millis, perSecond: entry.rate) {
        observers[id]?.last = millis
        passing.append(entry.handler)
      }
      return (published.lens, passing)
    }
    let orientation: CGImagePropertyOrientation = lens == .front ? .leftMirrored : .right
    let frame = CameraFrame(pixelBuffer: buffer, orientation: orientation, lens: lens)
    for handler in targets { handler(frame) }
  }
}
#endif
