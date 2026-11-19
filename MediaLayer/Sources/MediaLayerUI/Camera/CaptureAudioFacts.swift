#if os(iOS)
import AVFoundation
import CoreMedia
import Foundation
import MediaLayer

enum GapClock: Equatable, Sendable {
  case presentation
  case decode
}

struct TrackIntervals: Equatable, Sendable {
  var samples: Int
  var buffers: Int
  var firstPTS: CMTime
  var lastPTS: CMTime
  var durationMs: UInt64
  var maxGapMs: UInt64
  var gapClock: GapClock
  var overlaps: Int
  var maxBufferMs: UInt64
}

enum TrackFacts: Equatable, Sendable {
  case absent
  case empty
  case read(TrackIntervals)
  case failed(code: Int, message: String)
}

struct CaptureMediaFacts: Equatable, Sendable {
  var audio: TrackFacts
  var video: TrackFacts
}

func scanCaptureFacts(at url: URL) async throws -> CaptureMediaFacts {
  let task = Task.detached { try await scanFacts(of: AVURLAsset(url: url)) }
  return try await withTaskCancellationHandler {
    try await task.value
  } onCancel: {
    task.cancel()
  }
}

func scanAudioCoverage(at url: URL, required: Bool) async throws -> AudioCoverage {
  guard required else { return coverageFinish(scan: coverageBegin(config: CoverageConfig(audioRequired: false, audioTrack: false, sampleRate: 0, frameSamples: 0)), video: nil) }
  let task = Task.detached { try await readAudioCoverage(of: AVURLAsset(url: url)) }
  return try await withTaskCancellationHandler {
    try await task.value
  } onCancel: {
    task.cancel()
  }
}

private func readAudioCoverage(of asset: AVURLAsset) async throws -> AudioCoverage {
  let tracks: [AVAssetTrack]
  let video: TrackFacts
  do {
    tracks = try await asset.loadTracks(withMediaType: .audio)
    video = try await scanTrack(of: asset, mediaType: .video)
  } catch is CancellationError {
    throw CancellationError()
  } catch {
    return brokenCoverage()
  }
  // A camera clip always has video: a video that is absent, empty or unreadable is not an audio-only clip.
  guard case let .read(intervals) = video, let start = microseconds(intervals.firstPTS), let end = microseconds(intervals.lastPTS) else { return brokenCoverage() }
  guard let track = tracks.first else {
    return coverageFinish(scan: coverageBegin(config: CoverageConfig(audioRequired: true, audioTrack: false, sampleRate: 0, frameSamples: 0)), video: nil)
  }
  do {
    guard let stored = try await track.load(.formatDescriptions).first, let format = CMAudioFormatDescriptionGetStreamBasicDescription(stored)?.pointee,
          format.mSampleRate.isFinite, format.mSampleRate > 0, format.mSampleRate <= Double(UInt32.max), format.mChannelsPerFrame > 0,
          let scan = try await decodeCoverage(asset: asset, track: track, format: format) else { return brokenCoverage() }
    return coverageFinish(scan: scan, video: VideoBounds(startUs: start, endUs: end))
  } catch is CancellationError {
    throw CancellationError()
  } catch {
    return brokenCoverage()
  }
}

/// The packet table of the track, no PCM is decoded: every packet is one interval at its time with the samples its duration holds at the track rate, run through the core rule.
private func decodeCoverage(asset: AVURLAsset, track: AVAssetTrack, format stored: AudioStreamBasicDescription) async throws -> CoverageScan? {
  let reader = try AVAssetReader(asset: asset)
  let output = AVAssetReaderTrackOutput(track: track, outputSettings: nil)
  output.alwaysCopiesSampleData = false
  guard reader.canAdd(output) else { return nil }
  reader.add(output)
  guard reader.startReading() else { return nil }
  let rate = stored.mSampleRate
  var shiftUs: Int64 = 0
  var scan = coverageBegin(config: CoverageConfig(audioRequired: true, audioTrack: true, sampleRate: UInt32(rate.rounded()), frameSamples: stored.mFramesPerPacket))
  while let buffer = output.copyNextSampleBuffer() {
    if Task.isCancelled {
      reader.cancelReading()
      throw CancellationError()
    }
    guard let intervals = packetIntervals(of: buffer, rate: rate, shiftUs: &shiftUs) else { return nil }
    for interval in intervals { scan = coveragePush(scan: scan, interval: interval) }
  }
  guard reader.status == .completed else { return nil }
  return scan
}

// A buffer that holds no packet is nothing; a packet whose time or length cannot be told is an interval the core refuses.
private func packetIntervals(of buffer: CMSampleBuffer, rate: Double, shiftUs: inout Int64) -> [AudioInterval]? {
  let count = CMSampleBufferGetNumSamples(buffer)
  let refused = AudioInterval(startUs: 0, endUs: 1, samples: 0)
  guard CMSampleBufferIsValid(buffer), CMSampleBufferDataIsReady(buffer) else { return [refused] }
  guard count > 0 else { return CMSampleBufferGetTotalSampleSize(buffer) == 0 ? [] : [refused] }
  var needed = 0
  CMSampleBufferGetSampleTimingInfoArray(buffer, entryCount: 0, arrayToFill: nil, entriesNeededOut: &needed)
  var timings = [CMSampleTimingInfo](repeating: CMSampleTimingInfo(), count: needed)
  guard needed > 0, CMSampleBufferGetSampleTimingInfoArray(buffer, entryCount: needed, arrayToFill: &timings, entriesNeededOut: &needed) == noErr else { return [refused] }
  var result: [AudioInterval] = []
  for index in 0..<count {
    let timing = timings.count == count ? timings[index] : timings[0]
    let shift = timings.count == count ? CMTime.zero : CMTimeMultiply(timing.duration, multiplier: Int32(index))
    let pts = timing.presentationTimeStamp + shift
    guard timing.duration.isNumeric, timing.duration >= .zero else { return [refused] }
    guard timing.duration > .zero else { continue }
    guard let start = microseconds(pts), let end = microseconds(pts + timing.duration) else { return [refused] }
    result.append(AudioInterval(startUs: start, endUs: end, samples: UInt64((timing.duration.seconds * rate).rounded())))
  }
  return trimmed(result, by: buffer, rate: rate, shiftUs: &shiftUs)
}

/// The reader marks the priming of the encoder at the start and the padding at the end of a track as a trim of the buffer, the decoder never plays them and plays the track from its first heard sample: the table drops the same samples from the first and the last packets and moves every packet by the start trim.
private func trimmed(_ intervals: [AudioInterval], by buffer: CMSampleBuffer, rate: Double, shiftUs: inout Int64) -> [AudioInterval] {
  func trim(_ key: CFString) -> UInt64 {
    guard let raw = CMGetAttachment(buffer, key: key, attachmentModeOut: nil) else { return 0 }
    let time = CMTimeMakeFromDictionary(raw as! CFDictionary)
    return time.isNumeric && time > .zero ? UInt64((time.seconds * rate).rounded()) : 0
  }
  var list = intervals
  var head = trim(kCMSampleBufferAttachmentKey_TrimDurationAtStart)
  let removed = head
  while head > 0, let first = list.first {
    if head >= first.samples { head -= first.samples; list.removeFirst(); continue }
    let cut = Int64((Double(head) / rate * 1_000_000).rounded())
    list[0] = AudioInterval(startUs: first.startUs + cut, endUs: first.endUs, samples: first.samples - head)
    head = 0
  }
  shiftUs += Int64((Double(removed) / rate * 1_000_000).rounded())
  var tail = trim(kCMSampleBufferAttachmentKey_TrimDurationAtEnd)
  while tail > 0, let last = list.last {
    if tail >= last.samples { tail -= last.samples; list.removeLast(); continue }
    let cut = Int64((Double(tail) / rate * 1_000_000).rounded())
    list[list.count - 1] = AudioInterval(startUs: last.startUs, endUs: last.endUs - cut, samples: last.samples - tail)
    tail = 0
  }
  return list.map { AudioInterval(startUs: $0.startUs - shiftUs, endUs: $0.endUs - shiftUs, samples: $0.samples) }
}

private func brokenCoverage() -> AudioCoverage {
  coverageFinish(scan: coverageBegin(config: CoverageConfig(audioRequired: true, audioTrack: true, sampleRate: 0, frameSamples: 0)), video: nil)
}

private func microseconds(_ time: CMTime) -> Int64? {
  guard time.isNumeric else { return nil }
  let converted = CMTimeConvertScale(time, timescale: 1_000_000, method: .roundHalfAwayFromZero)
  return converted.isNumeric ? converted.value : nil
}

private func scanFacts(of asset: AVURLAsset) async throws -> CaptureMediaFacts {
  async let audio = scanTrack(of: asset, mediaType: .audio)
  async let video = scanTrack(of: asset, mediaType: .video)
  return try await CaptureMediaFacts(audio: audio, video: video)
}

private func scanTrack(of asset: AVURLAsset, mediaType: AVMediaType) async throws -> TrackFacts {
  let tracks: [AVAssetTrack]
  do { tracks = try await asset.loadTracks(withMediaType: mediaType) }
  catch is CancellationError { throw CancellationError() }
  catch { return failure(error) }
  guard let track = tracks.first else { return .absent }
  do {
    let reader = try AVAssetReader(asset: asset)
    let output = AVAssetReaderTrackOutput(track: track, outputSettings: nil)
    output.alwaysCopiesSampleData = false
    guard reader.canAdd(output) else { return .failed(code: 0, message: "track output rejected") }
    reader.add(output)
    guard reader.startReading() else { return reader.error.map(failure) ?? .failed(code: 0, message: "reading did not start") }
    var scan = IntervalScan(gapClock: mediaType == .audio ? .presentation : .decode)
    while let buffer = output.copyNextSampleBuffer() {
      if Task.isCancelled {
        reader.cancelReading()
        throw CancellationError()
      }
      if let message = scan.add(buffer) {
        reader.cancelReading()
        return .failed(code: 0, message: message)
      }
    }
    guard reader.status == .completed else { return failure(reader.error) }
    return scan.facts
  } catch is CancellationError {
    throw CancellationError()
  } catch {
    return failure(error)
  }
}

private func failure(_ error: Error?) -> TrackFacts {
  let native = error as NSError?
  return .failed(code: native?.code ?? 0, message: native?.localizedDescription ?? "reading failed")
}

private struct IntervalScan {
  let gapClock: GapClock
  var samples = 0
  var buffers = 0
  var firstPTS = CMTime.invalid
  var lastEnd = CMTime.invalid
  var previousEnd = CMTime.invalid
  var maxGap = CMTime.zero
  var overlaps = 0
  var maxBuffer = CMTime.zero

  mutating func add(_ buffer: CMSampleBuffer) -> String? {
    let count = CMSampleBufferGetNumSamples(buffer)
    let pts = CMSampleBufferGetOutputPresentationTimeStamp(buffer)
    let duration = CMSampleBufferGetOutputDuration(buffer)
    guard CMSampleBufferIsValid(buffer), CMSampleBufferDataIsReady(buffer) else { return "sample buffer is not valid or its data is not ready" }
    guard count > 0 else { return CMSampleBufferGetTotalSampleSize(buffer) == 0 ? nil : "sample buffer with data and without samples" }
    guard pts.isNumeric else { return "sample buffer without a numeric output presentation time" }
    guard duration.isNumeric, duration >= .zero else { return "sample buffer without a valid output duration" }
    guard duration > .zero else { return nil }
    let decode = CMSampleBufferGetDecodeTimeStamp(buffer)
    let start = gapClock == .decode && decode.isNumeric ? decode : pts
    samples += count
    buffers += 1
    maxBuffer = max(maxBuffer, duration)
    if previousEnd.isValid {
      let gap = start - previousEnd
      if gap < .zero { overlaps += 1 } else { maxGap = max(maxGap, gap) }
    }
    previousEnd = start + duration
    firstPTS = firstPTS.isValid ? min(firstPTS, pts) : pts
    lastEnd = lastEnd.isValid ? max(lastEnd, pts + duration) : pts + duration
    return nil
  }

  var facts: TrackFacts {
    guard buffers > 0 else { return .empty }
    guard let duration = milliseconds(lastEnd - firstPTS), let gap = milliseconds(maxGap), let buffer = milliseconds(maxBuffer) else {
      return .failed(code: 0, message: "interval out of range")
    }
    return .read(TrackIntervals(
      samples: samples, buffers: buffers, firstPTS: firstPTS, lastPTS: lastEnd,
      durationMs: duration, maxGapMs: gap, gapClock: gapClock, overlaps: overlaps, maxBufferMs: buffer
    ))
  }
}

private func milliseconds(_ interval: CMTime) -> UInt64? {
  let seconds = interval.seconds
  guard interval.isNumeric, seconds.isFinite, seconds >= 0, seconds * 1000 < Double(UInt64.max) else { return nil }
  return UInt64(seconds * 1000)
}
#endif
