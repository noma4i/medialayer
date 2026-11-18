@preconcurrency import AVFoundation
import CoreMedia
import Foundation
import MediaLayer

/// One voice take: the PCM of the first live generation written to a file by the writer, which is opened by the first live buffer. It works on the queue of its source. A buffer the writer cannot take yet waits retained in a queue and goes in when the writer is ready; the pool of the capture is the only bound: a buffer it dropped is a hole in the sound and ends the take.
final class VoiceTake: VoicePCMSink, @unchecked Sendable {
  enum Failure: Error {
    case writerRejected
    case appendFailed
    case audioBehind
    case invalidInterval
  }

  /// How a take ends by itself, told once.
  enum Broken {
    case lost(Error?)
    case failed(Error)
  }

  private enum Phase { case waiting, writing, finishing, finishingCancelled, settled }

  let url: URL
  private let writer: VoiceFileWriter
  private let settings: [String: Any]
  private let maxDurationMs: UInt64
  private let onStarted: @Sendable () -> Void
  private let onLimit: @Sendable () -> Void
  private let onBroken: @Sendable (Broken) -> Void
  private let lock = NSLock()
  private var phase = Phase.waiting
  private var bound: UInt64?
  private var start = CMTime.invalid
  private var end = CMTime.invalid
  private var pending: [CMSampleBuffer] = []
  private var limited = false
  private var peak: Float = 0

  init(url: URL, writer: VoiceFileWriter, settings: [String: Any], maxDurationMs: UInt64, onStarted: @escaping @Sendable () -> Void, onLimit: @escaping @Sendable () -> Void, onBroken: @escaping @Sendable (Broken) -> Void) {
    self.url = url
    self.writer = writer
    self.settings = settings
    self.maxDurationMs = maxDurationMs
    self.onStarted = onStarted
    self.onLimit = onLimit
    self.onBroken = onBroken
  }

  func takePeak() -> Float { lock.withLock { defer { peak = 0 }; return peak } }

  func pcm(generation: UInt64, _ buffer: CMSampleBuffer) {
    enum Event { case none, started, limit, failed(Error) }
    let event = lock.withLock { () -> Event in
      guard phase == .waiting || phase == .writing, !limited, bound == nil || bound == generation, isLivePCM(buffer) else { return .none }
      do {
        let opened = phase == .waiting
        if opened {
          try writer.open(first: buffer, settings: settings)
          start = CMSampleBufferGetPresentationTimeStamp(buffer)
          bound = generation
          phase = .writing
        }
        pending.append(buffer)
        try drain()
        peak = max(peak, Self.linearPeak(of: buffer))
        if limited { return .limit }
        return opened ? .started : .none
      } catch {
        discard()
        return .failed(error)
      }
    }
    switch event {
    case .none: break
    case .started: onStarted()
    case .limit:
      onStarted()
      onLimit()
    case let .failed(error): onBroken(.failed(error))
    }
  }

  /// The capture dropped a buffer: the sound has a hole, the take ends with `audioBehind`.
  func dropped(generation: UInt64) {
    let report = lock.withLock { () -> Bool in
      guard phase == .waiting || phase == .writing, bound == nil || bound == generation else { return false }
      discard()
      return true
    }
    if report { onBroken(.failed(Failure.audioBehind)) }
  }

  func lost(generation: UInt64, error: Error?) {
    let report = lock.withLock { () -> Bool in
      guard phase == .waiting || phase == .writing, bound == nil || bound == generation else { return false }
      discard()
      return true
    }
    if report { onBroken(.lost(error)) }
  }

  /// Closes the file of a take that is writing and returns the length of the written audio in microseconds: from the first written PCM to the end of the last sample. A take that never wrote returns 0. A cancel that comes while the file closes removes the file after the native completion and the result is the cancel.
  @discardableResult
  func finish() async throws -> UInt64 {
    let writing = lock.withLock { () -> Bool in
      guard phase == .writing else {
        if phase == .waiting { phase = .settled }
        return false
      }
      phase = .finishing
      return true
    }
    guard writing else { return 0 }
    var failure: Error?
    do {
      try await flush()
      try await writer.finish()
    } catch { failure = error }
    let cancelled = lock.withLock { () -> Bool in
      defer { phase = .settled }
      return phase == .finishingCancelled
    }
    if cancelled || failure != nil { NativeFailures.removeFile(url) }
    if cancelled { throw CancellationError() }
    if let failure { throw failure }
    return lock.withLock { Self.microseconds(from: start, to: end) }
  }

  private static func microseconds(from start: CMTime, to end: CMTime) -> UInt64 {
    let seconds = (end - start).seconds
    return seconds.isFinite && seconds > 0 ? UInt64((seconds * 1_000_000).rounded()) : 0
  }

  func cancel() {
    lock.withLock { discard() }
  }

  /// Goes in while the writer is ready; the rest waits for the next buffer or for `flush`.
  private func drain() throws {
    while let buffer = pending.first, writer.isReady {
      pending.removeFirst()
      try append(buffer)
    }
  }

  /// At the end every waiting buffer goes in before the file closes; a cancel or a failed writer ends the wait.
  private func flush() async throws {
    while true {
      let state = try lock.withLock { () -> Bool? in
        guard phase == .finishing, !pending.isEmpty else { return false }
        if let error = writer.failure { throw error }
        try drain()
        return pending.isEmpty ? false : nil
      }
      if state == false { return }
      await Task.yield()
      try await Task.sleep(nanoseconds: 2_000_000)
    }
  }

  private func append(_ buffer: CMSampleBuffer) throws {
    try writer.append(buffer)
    let tail = CMSampleBufferGetPresentationTimeStamp(buffer) + CMSampleBufferGetDuration(buffer)
    let elapsed = (tail - start).seconds
    guard (tail - start).isNumeric, elapsed.isFinite, elapsed >= 0 else { throw Failure.invalidInterval }
    end = tail
    if elapsed * 1000 >= Double(maxDurationMs) { limited = true }
  }

  private func discard() {
    switch phase {
    case .settled, .finishingCancelled: return
    case .finishing:
      pending = []
      phase = .finishingCancelled
      return
    case .waiting: phase = .settled
    case .writing:
      pending = []
      phase = .settled
      writer.cancel()
    }
    NativeFailures.removeFile(url)
  }

  private static func linearPeak(of buffer: CMSampleBuffer) -> Float {
    guard let block = CMSampleBufferGetDataBuffer(buffer), let format = CMSampleBufferGetFormatDescription(buffer),
          let description = CMAudioFormatDescriptionGetStreamBasicDescription(format)?.pointee else { return 0 }
    var length = 0
    var total = 0
    var pointer: UnsafeMutablePointer<Int8>?
    guard CMBlockBufferGetDataPointer(block, atOffset: 0, lengthAtOffsetOut: &length, totalLengthOut: &total, dataPointerOut: &pointer) == noErr, let pointer else { return 0 }
    let raw = UnsafeRawBufferPointer(start: pointer, count: length)
    if description.mFormatFlags & kAudioFormatFlagIsFloat != 0, description.mBitsPerChannel == 32 {
      return raw.bindMemory(to: Float.self).reduce(0) { max($0, abs($1)) }.clamped
    }
    guard description.mBitsPerChannel == 16 else { return 0 }
    return Float(raw.bindMemory(to: Int16.self).reduce(0) { max($0, abs(Int32($1))) }) / 32_768
  }
}

private extension Float {
  var clamped: Float { isNaN ? 0 : min(self, 1) }
}
