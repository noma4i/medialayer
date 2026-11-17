#if os(iOS)
import AVFoundation
import CoreMedia
import Foundation
import MediaLayer

// Held audio is the first sample written: onBegin waits for the core confirm, and only audio may open the writer when the profile has audio.
final class CameraTake: @unchecked Sendable {
  struct Settings: @unchecked Sendable {
    var video: [String: Any]
    var audio: [String: Any]?
    var transform: CGAffineTransform
    var maxDurationMs: UInt64
    var fileType: AVFileType
    var fragmentInterval: CMTime
    var armPTS: CMTime
    var maxPendingAudio: Int
    var maxPendingVideo: Int
    var maxHeldAudio: Int
    var memoryBudgetBytes: Int
  }

  enum Outcome {
    case finished(url: URL, durationMs: UInt64, reachedLimit: Bool)
    case cancelled
    case failed(Error)
  }

  enum Failure: Error {
    case writerRejected
    case appendFailed
    case audioBacklog
    case audioBehind
    case memoryBudget
    case invalidBuffer
    case missingStream
    case invalidInterval
  }

  let generation: UInt64
  let url: URL
  private let settings: Settings
  private let onBegin: @Sendable (UInt64, CMTime) -> Void
  private let onStarted: @Sendable () -> Void
  private let onOutcome: @Sendable (Outcome) -> Void
  private let queue = DispatchQueue(label: "com.noma4i.medialayer.camera.take", qos: .userInitiated)
  private let lock = NSLock()
  private var pendingAudio = 0
  private var pendingVideo = 0
  private var reservedBytes = 0
  private var refused = false
  private var pendingOutcome: Outcome?

  private enum Phase { case waiting, held, writing, finishing, settled }
  private var phase = Phase.waiting
  private var held: [(buffer: CMSampleBuffer, bytes: Int)] = []
  private var writer: AVAssetWriter?
  private var audioInput: AVAssetWriterInput?
  private var videoInput: AVAssetWriterInput?
  private var start = CMTime.invalid
  private var end = CMTime.invalid
  private var audioCount = 0
  private var videoCount = 0
  private var writtenMs: UInt64 = 0

  /// The length of the clock stream written so far, for the ticker of the camera.
  var recordedMs: UInt64 { lock.withLock { writtenMs } }

  init(
    url: URL, generation: UInt64, settings: Settings,
    onBegin: @escaping @Sendable (UInt64, CMTime) -> Void,
    onStarted: @escaping @Sendable () -> Void,
    onOutcome: @escaping @Sendable (Outcome) -> Void
  ) {
    self.url = url
    self.generation = generation
    self.settings = settings
    self.onBegin = onBegin
    self.onStarted = onStarted
    self.onOutcome = onOutcome
  }

  func appendAudio(_ buffer: CMSampleBuffer, generation: UInt64) {
    guard generation == self.generation, settings.audio != nil, isFresh(buffer, mediaType: kCMMediaType_Audio, subType: kAudioFormatLinearPCM) else { return }
    let bytes = Self.bytes(buffer)
    switch admit(audio: true, bytes: bytes) {
    case .taken:
      queue.async { [self] in
        let retained = handleAudio(buffer, bytes: bytes)
        releaseSlot(audio: true)
        releaseBytes(retained ? 0 : bytes)
      }
    case .refused(let failure):
      queue.async { [self] in settle(.failed(failure)) }
    case .dropped:
      break
    }
  }

  func appendVideo(_ buffer: CMSampleBuffer, generation: UInt64) {
    guard generation == self.generation, isFresh(buffer, mediaType: kCMMediaType_Video, subType: nil) else { return }
    let bytes = Self.bytes(buffer)
    guard case .taken = admit(audio: false, bytes: bytes) else { return }
    queue.async { [self] in
      handleVideo(buffer)
      releaseSlot(audio: false)
      releaseBytes(bytes)
    }
  }

  func confirmBegin(generation: UInt64) {
    guard generation == self.generation else { return }
    queue.async { [self] in
      guard phase == .held else { return }
      open(first: held[0].buffer)
    }
  }

  func stop() {
    queue.async { [self] in finish(reachedLimit: false) }
  }

  func cancel() {
    queue.async { [self] in settle(.cancelled) }
  }

  private func handleAudio(_ buffer: CMSampleBuffer, bytes: Int) -> Bool {
    switch phase {
    case .waiting:
      held = [(buffer, bytes)]
      phase = .held
      onBegin(generation, CMSampleBufferGetPresentationTimeStamp(buffer))
      return true
    case .held:
      guard held.count < settings.maxHeldAudio else {
        settle(.failed(Failure.audioBacklog))
        return false
      }
      held.append((buffer, bytes))
      return true
    case .writing:
      appendWritten(buffer, to: audioInput, isAudio: true)
    case .finishing, .settled:
      break
    }
    return false
  }

  private func handleVideo(_ buffer: CMSampleBuffer) {
    if phase == .waiting, settings.audio == nil { return open(first: buffer) }
    guard phase == .writing, CMSampleBufferGetPresentationTimeStamp(buffer) >= start else { return }
    appendWritten(buffer, to: videoInput, isAudio: false)
  }

  private func open(first: CMSampleBuffer) {
    do {
      let writer = try AVAssetWriter(outputURL: url, fileType: settings.fileType)
      writer.movieFragmentInterval = settings.fragmentInterval
      let video = AVAssetWriterInput(mediaType: .video, outputSettings: settings.video)
      video.expectsMediaDataInRealTime = true
      video.transform = settings.transform
      guard writer.canAdd(video) else { throw Failure.writerRejected }
      writer.add(video)
      videoInput = video
      if let audioSettings = settings.audio {
        let audio = AVAssetWriterInput(mediaType: .audio, outputSettings: audioSettings, sourceFormatHint: CMSampleBufferGetFormatDescription(first))
        audio.expectsMediaDataInRealTime = true
        guard writer.canAdd(audio) else { throw Failure.writerRejected }
        writer.add(audio)
        audioInput = audio
      }
      guard writer.startWriting() else { throw writer.error ?? Failure.writerRejected }
      self.writer = writer
      start = CMSampleBufferGetPresentationTimeStamp(first)
      writer.startSession(atSourceTime: start)
      phase = .writing
      if settings.audio != nil {
        let flushed = held
        held = []
        for (index, sample) in flushed.enumerated() {
          if phase == .writing { appendWritten(sample.buffer, to: audioInput, isAudio: true) }
          releaseBytes(sample.bytes)
          if index == 0, phase == .writing { onStarted() }
        }
      } else {
        appendWritten(first, to: videoInput, isAudio: false)
        if phase == .writing { onStarted() }
      }
    } catch {
      settle(.failed(error))
    }
  }

  private func appendWritten(_ buffer: CMSampleBuffer, to input: AVAssetWriterInput?, isAudio: Bool) {
    guard phase == .writing, let input else { return }
    let pts = CMSampleBufferGetPresentationTimeStamp(buffer)
    let carriesClock = isAudio || settings.audio == nil
    if carriesClock, settings.maxDurationMs > 0, let elapsed = Self.milliseconds(pts - start), elapsed >= settings.maxDurationMs {
      return finish(reachedLimit: true)
    }
    guard input.isReadyForMoreMediaData else {
      if isAudio { settle(.failed(Failure.audioBehind)) }
      return
    }
    guard input.append(buffer) else { return settle(.failed(writer?.error ?? Failure.appendFailed)) }
    if isAudio { audioCount += 1 } else { videoCount += 1 }
    if carriesClock {
      let tail = pts + CMSampleBufferGetDuration(buffer)
      end = end.isValid ? max(end, tail) : tail
      if let elapsed = Self.milliseconds(end - start) { lock.withLock { writtenMs = elapsed } }
    }
  }

  private func finish(reachedLimit: Bool) {
    switch phase {
    case .finishing, .settled: return
    case .waiting, .held: return settle(.failed(Failure.missingStream))
    case .writing: break
    }
    guard let writer else { return settle(.failed(Failure.missingStream)) }
    guard videoCount > 0, settings.audio == nil || audioCount > 0 else { return settle(.failed(Failure.missingStream)) }
    guard let duration = Self.milliseconds(end - start) else { return settle(.failed(Failure.invalidInterval)) }
    phase = .finishing
    audioInput?.markAsFinished()
    videoInput?.markAsFinished()
    writer.finishWriting { [self] in
      queue.async { [self] in
        guard phase == .finishing, let writer = self.writer else { return }
        if let pendingOutcome { return conclude(pendingOutcome) }
        guard writer.status == .completed else { return conclude(.failed(writer.error ?? Failure.appendFailed)) }
        conclude(.finished(url: url, durationMs: duration, reachedLimit: reachedLimit))
      }
    }
  }

  private func settle(_ outcome: Outcome) {
    guard phase != .finishing else {
      if pendingOutcome == nil { pendingOutcome = outcome }
      return
    }
    conclude(outcome)
  }

  private func conclude(_ outcome: Outcome) {
    guard phase != .settled else { return }
    phase = .settled
    for sample in held { releaseBytes(sample.bytes) }
    held = []
    if case .finished = outcome {} else {
      if let writer, writer.status == .writing { writer.cancelWriting() }
      NativeFailures.removeFile(url)
    }
    audioInput = nil
    videoInput = nil
    writer = nil
    onOutcome(outcome)
  }

  private enum Admission { case taken, dropped, refused(Failure) }

  private func admit(audio: Bool, bytes: Int) -> Admission {
    lock.withLock {
      guard !refused else { return .dropped }
      let failure: Failure?
      if bytes <= 0 { failure = .invalidBuffer }
      else if reservedBytes + bytes > settings.memoryBudgetBytes { failure = .memoryBudget }
      else if audio ? pendingAudio >= settings.maxPendingAudio : pendingVideo >= settings.maxPendingVideo { failure = .audioBacklog }
      else { failure = nil }
      if let failure {
        guard audio else { return .dropped }
        refused = true
        return .refused(failure)
      }
      if audio { pendingAudio += 1 } else { pendingVideo += 1 }
      reservedBytes += bytes
      return .taken
    }
  }

  private func releaseSlot(audio: Bool) {
    lock.withLock { if audio { pendingAudio -= 1 } else { pendingVideo -= 1 } }
  }

  private func releaseBytes(_ bytes: Int) {
    lock.withLock { reservedBytes -= bytes }
  }

  private func isFresh(_ buffer: CMSampleBuffer, mediaType: CMMediaType, subType: FourCharCode?) -> Bool {
    guard CMSampleBufferIsValid(buffer), CMSampleBufferDataIsReady(buffer), CMSampleBufferGetNumSamples(buffer) > 0,
          let format = CMSampleBufferGetFormatDescription(buffer), CMFormatDescriptionGetMediaType(format) == mediaType,
          subType.map({ CMFormatDescriptionGetMediaSubType(format) == $0 }) ?? true else { return false }
    let pts = CMSampleBufferGetPresentationTimeStamp(buffer)
    let duration = CMSampleBufferGetDuration(buffer)
    guard pts.isNumeric, pts >= settings.armPTS, mediaType != kCMMediaType_Audio || (duration.isNumeric && duration > .zero) else { return false }
    return true
  }

  private static func bytes(_ buffer: CMSampleBuffer) -> Int {
    if let image = CMSampleBufferGetImageBuffer(buffer) { return CVPixelBufferGetDataSize(image) }
    return CMSampleBufferGetTotalSampleSize(buffer)
  }

  private static func milliseconds(_ interval: CMTime) -> UInt64? {
    let seconds = interval.seconds
    guard interval.isNumeric, seconds.isFinite, seconds >= 0, seconds * 1000 < Double(UInt64.max) else { return nil }
    return UInt64(seconds * 1000)
  }
}
#endif
