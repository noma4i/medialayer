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
    case finished(url: URL, durationMs: UInt64, end: RecordEnd, coverage: AudioCoverage)
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

  private enum Phase { case waiting, held, writing, stopping, finishing, settled }
  private var phase = Phase.waiting
  private var held: [(buffer: CMSampleBuffer, bytes: Int)] = []
  private var writer: AVAssetWriter?
  private var audioInput: AVAssetWriterInput?
  private var videoInput: AVAssetWriterInput?
  private var start = CMTime.invalid
  private var end = CMTime.invalid
  private var videoEnd = CMTime.invalid
  private var videoPTS = CMTime.invalid
  private var audioEnd = CMTime.invalid
  private var sourceEnded = false
  private var recordEnd = RecordEnd.completed
  private var ledger = TakeLedger(audio: nil)
  private var audioCount = 0
  private var videoCount = 0
  private var writtenMs: UInt64 = 0
  /// Audio callbacks by outcome, for the trace line at the end of the take; guarded by `lock`.
  private struct AudioTrace {
    var taken = 0, early = 0, badDuration = 0, notFresh = 0, generation = 0, dropped = 0, notReady = 0
    var lastArrivedUs: Int64 = -1
  }
  private var audioTrace = AudioTrace()

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
    guard settings.audio != nil else { return }
    guard generation == self.generation else { return lock.withLock { audioTrace.generation += 1 } }
    let arrived = microseconds(CMSampleBufferGetPresentationTimeStamp(buffer))
    let reason = freshness(buffer, mediaType: kCMMediaType_Audio, subType: kAudioFormatLinearPCM)
    lock.withLock {
      if let arrived { audioTrace.lastArrivedUs = arrived }
      switch reason {
      case .fresh: break
      case .early: audioTrace.early += 1
      case .badDuration: audioTrace.badDuration += 1
      case .other: audioTrace.notFresh += 1
      }
    }
    guard reason == .fresh else { return }
    let bytes = Self.bytes(buffer)
    switch admit(audio: true, bytes: bytes) {
    case .taken:
      lock.withLock { audioTrace.taken += 1 }
      queue.async { [self] in
        let retained = handleAudio(buffer, bytes: bytes)
        releaseSlot(audio: true)
        releaseBytes(retained ? 0 : bytes)
      }
    case .refused(let failure):
      queue.async { [self] in settle(.failed(failure)) }
    case .dropped:
      lock.withLock { audioTrace.dropped += 1 }
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
    queue.async { [self] in finish(end: .completed) }
  }

  func cancel() {
    queue.async { [self] in settle(.cancelled) }
  }

  /// The sound source delivers no more samples: a stop that waits for the sound ends with what was written.
  func audioEnded() {
    queue.async { [self] in
      sourceEnded = true
      if phase == .stopping { endSession() }
    }
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
    case .stopping:
      appendWritten(buffer, to: audioInput, isAudio: true)
      if phase == .stopping { endIfCovered() }
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
        ledger = TakeLedger(audio: TakeLedger.audioFormat(of: first) ?? (rate: 0, frameSamples: 0))
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
    guard phase == .writing || phase == .stopping, let input else { return }
    let pts = CMSampleBufferGetPresentationTimeStamp(buffer)
    let carriesClock = isAudio || settings.audio == nil
    if carriesClock, phase == .writing, settings.maxDurationMs > 0, let elapsed = Self.milliseconds(pts - start), elapsed >= settings.maxDurationMs {
      return finish(end: .limitReached)
    }
    guard input.isReadyForMoreMediaData else {
      if isAudio {
        lock.withLock { audioTrace.notReady += 1 }
        settle(.failed(Failure.audioBehind))
      }
      return
    }
    guard input.append(buffer) else { return settle(.failed(writer?.error ?? Failure.appendFailed)) }
    if isAudio {
      audioCount += 1
      ledger.audio(TakeLedger.interval(of: buffer))
      let tail = pts + CMSampleBufferGetDuration(buffer)
      audioEnd = audioEnd.isValid ? max(audioEnd, tail) : tail
    } else {
      if videoCount == 0, let image = CMSampleBufferGetImageBuffer(buffer) {
        MediaTrace.emit("camera", "take first frame width=\(CVPixelBufferGetWidth(image)) height=\(CVPixelBufferGetHeight(image)) writer=\(settings.video[AVVideoWidthKey] ?? "?")x\(settings.video[AVVideoHeightKey] ?? "?")")
      }
      videoCount += 1
      videoEnd = Self.frameEnd(pts: pts, duration: CMSampleBufferGetDuration(buffer), previous: videoPTS)
      videoPTS = pts
      if let first = microseconds(pts), let last = microseconds(videoEnd) { ledger.video(startUs: first, endUs: last) }
      if settings.audio != nil, phase == .writing, endIfMicrophoneLost() { return }
    }
    if carriesClock {
      let tail = pts + CMSampleBufferGetDuration(buffer)
      end = end.isValid ? max(end, tail) : tail
      if let elapsed = Self.milliseconds(end - start) { lock.withLock { writtenMs = elapsed } }
    }
  }

  /// The sound source stopped delivering while the picture goes on: the take ends at the last sound written, and the video bound of the ledger ends there too (INV-CAPTURE-80).
  private func endIfMicrophoneLost() -> Bool {
    let audioUs = audioEnd.isValid ? microseconds(audioEnd - start) ?? -1 : -1
    guard let videoUs = microseconds(videoEnd - start), microphoneLost(audioEndUs: audioUs, videoEndUs: videoUs), audioEnd.isValid else { return false }
    MediaTrace.emit("camera", "take microphone lost audioEndUs=\(audioUs) videoEndUs=\(videoUs)")
    videoEnd = audioEnd
    if let bound = microseconds(audioEnd) { ledger.clipVideo(to: bound) }
    finish(end: .microphoneLost)
    return true
  }

  private func finish(end: RecordEnd) {
    switch phase {
    case .stopping, .finishing, .settled: return
    case .waiting, .held: return settle(.failed(Failure.missingStream))
    case .writing: break
    }
    guard let writer else { return settle(.failed(Failure.missingStream)) }
    guard videoCount > 0, settings.audio == nil || audioCount > 0 else { return settle(.failed(Failure.missingStream)) }
    guard Self.milliseconds(videoEnd - start) != nil else { return settle(.failed(Failure.invalidInterval)) }
    recordEnd = end
    phase = .stopping
    endIfCovered()
  }

  /// The tracks end at the last video frame; the end waits until the written sound covers it or the sound source has ended, with no timer (INV-CAPTURE-51, INV-CAPTURE-80).
  private func endIfCovered() {
    guard phase == .stopping else { return }
    let covered = settings.audio == nil || sourceEnded || (audioEnd.isValid && audioEnd >= videoEnd)
    if covered { endSession() }
  }

  private func endSession() {
    guard phase == .stopping, let writer, let duration = Self.milliseconds(videoEnd - start) else { return }
    let recordEnd = self.recordEnd
    phase = .finishing
    audioInput?.markAsFinished()
    videoInput?.markAsFinished()
    writer.endSession(atSourceTime: videoEnd)
    writer.finishWriting { [self] in
      queue.async { [self] in
        guard phase == .finishing, let writer = self.writer else { return }
        if let pendingOutcome { return conclude(pendingOutcome) }
        guard writer.status == .completed else { return conclude(.failed(writer.error ?? Failure.appendFailed)) }
        conclude(.finished(url: url, durationMs: duration, end: recordEnd, coverage: ledger.coverage()))
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
    if settings.audio != nil {
      let t = lock.withLock { audioTrace }
      let written = microseconds(audioEnd).map { String($0) } ?? "none"
      MediaTrace.emit("camera", "take audio taken=\(t.taken) notFresh=\(t.notFresh) early=\(t.early) badDuration=\(t.badDuration) generation=\(t.generation) dropped=\(t.dropped) notReady=\(t.notReady) lastArrivedUs=\(t.lastArrivedUs) lastWrittenUs=\(written)")
    }
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

  private enum Freshness { case fresh, early, badDuration, other }

  private func isFresh(_ buffer: CMSampleBuffer, mediaType: CMMediaType, subType: FourCharCode?) -> Bool {
    freshness(buffer, mediaType: mediaType, subType: subType) == .fresh
  }

  private func freshness(_ buffer: CMSampleBuffer, mediaType: CMMediaType, subType: FourCharCode?) -> Freshness {
    guard CMSampleBufferIsValid(buffer), CMSampleBufferDataIsReady(buffer), CMSampleBufferGetNumSamples(buffer) > 0,
          let format = CMSampleBufferGetFormatDescription(buffer), CMFormatDescriptionGetMediaType(format) == mediaType,
          subType.map({ CMFormatDescriptionGetMediaSubType(format) == $0 }) ?? true else { return .other }
    let pts = CMSampleBufferGetPresentationTimeStamp(buffer)
    let duration = CMSampleBufferGetDuration(buffer)
    guard pts.isNumeric else { return .other }
    guard pts >= settings.armPTS else { return .early }
    guard mediaType != kCMMediaType_Audio || (duration.isNumeric && duration > .zero) else { return .badDuration }
    return .fresh
  }

  static let entryCost = MemoryLayout<(buffer: CMSampleBuffer, bytes: Int)>.stride

  private static func bytes(_ buffer: CMSampleBuffer) -> Int {
    let payload = CMSampleBufferGetImageBuffer(buffer).map(CVPixelBufferGetDataSize) ?? CMSampleBufferGetTotalSampleSize(buffer)
    return payload > 0 ? payload + entryCost : 0
  }

  /// The end of a written video frame: its pts and duration, or the gap since the previous frame when the camera gives no duration.
  /// The tracks of the take end here, so the sound never runs past the last picture (INV-CAPTURE-51).
  static func frameEnd(pts: CMTime, duration: CMTime, previous: CMTime) -> CMTime {
    if duration.isNumeric, duration > .zero { return pts + duration }
    if previous.isNumeric, pts > previous { return pts + (pts - previous) }
    return pts
  }

  private static func milliseconds(_ interval: CMTime) -> UInt64? {
    let seconds = interval.seconds
    guard interval.isNumeric, seconds.isFinite, seconds >= 0, seconds * 1000 < Double(UInt64.max) else { return nil }
    return UInt64(seconds * 1000)
  }
}
#endif
