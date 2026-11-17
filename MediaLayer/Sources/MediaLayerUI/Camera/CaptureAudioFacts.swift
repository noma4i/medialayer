#if os(iOS)
import AVFoundation
import CoreMedia
import Foundation

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
