#if os(iOS)
import AVFoundation
import CoreMedia
import MediaLayer

/// What the writer of a take wrote, kept as it writes: every audio sample at the single write point and the bounds of the video.
/// The verdict is the core rule of the file scan (`coveragePush`, `coverageFinish`), so a take is judged while it records and the file is not read back after stop.
struct TakeLedger: Sendable {
  private var scan: CoverageScan
  private var firstVideoUs: Int64?
  private var videoEndUs: Int64?

  /// `audio` is the PCM rate and the native codec frame of a take with sound; nil is a take without sound in its profile.
  init(audio: (rate: UInt32, frameSamples: UInt32)?) {
    let config = audio.map { CoverageConfig(audioRequired: true, audioTrack: true, sampleRate: $0.rate, frameSamples: $0.frameSamples) }
      ?? CoverageConfig(audioRequired: false, audioTrack: false, sampleRate: 0, frameSamples: 0)
    scan = coverageBegin(config: config)
  }

  mutating func audio(_ interval: AudioInterval) {
    scan = coveragePush(scan: scan, interval: interval)
  }

  mutating func video(startUs: Int64, endUs: Int64) {
    firstVideoUs = firstVideoUs ?? startUs
    videoEndUs = max(videoEndUs ?? endUs, endUs)
  }

  /// The video of the take ends at `endUs`: the picture past the last sound is not part of the take.
  mutating func clipVideo(to endUs: Int64) {
    videoEndUs = videoEndUs.map { min($0, endUs) }
  }

  func coverage() -> AudioCoverage {
    coverageFinish(scan: scan, video: VideoBounds(startUs: firstVideoUs ?? 0, endUs: videoEndUs ?? 0))
  }

  /// The interval of one PCM buffer on the timeline of the take: its time and the samples it holds.
  /// A buffer whose time cannot be told is an interval the core refuses.
  static func interval(of buffer: CMSampleBuffer) -> AudioInterval {
    let pts = CMSampleBufferGetPresentationTimeStamp(buffer)
    guard let start = microseconds(pts), let end = microseconds(pts + CMSampleBufferGetDuration(buffer)) else { return AudioInterval(startUs: 0, endUs: 1, samples: 0) }
    return AudioInterval(startUs: start, endUs: end, samples: UInt64(max(0, CMSampleBufferGetNumSamples(buffer))))
  }

  /// The rate of the PCM a take writes and the samples one packet of its encoder holds (AAC: 1024).
  static func audioFormat(of buffer: CMSampleBuffer) -> (rate: UInt32, frameSamples: UInt32)? {
    guard let format = CMSampleBufferGetFormatDescription(buffer), let stream = CMAudioFormatDescriptionGetStreamBasicDescription(format)?.pointee,
          stream.mSampleRate.isFinite, stream.mSampleRate > 0, stream.mSampleRate <= Double(UInt32.max) else { return nil }
    return (UInt32(stream.mSampleRate.rounded()), aacFrameSamples)
  }

  static let aacFrameSamples: UInt32 = 1024
}
#endif
