#if os(iOS)
@preconcurrency import AVFoundation
import Foundation
import MediaLayer

/// The upright pass of a take: the system writer stores the turn of the sensor as a matrix, this pass turns the pixels so the file the camera delivers has an identity matrix.
/// The front mirror of the profile is applied in the same pass, in pixels.
enum CaptureUpright {
  /// Runs after the system writer finished `url`. Nothing is written when the take already is upright and not mirrored. A failure fails the take.
  static func run(_ url: URL, mirror: Bool) async throws {
    let started = DispatchTime.now().uptimeNanoseconds
    let asset = PrepareContainer.asset(url)
    guard let track = try await asset.loadTracks(withMediaType: .video).first else { return }
    let (size, transform, rate, frameRate, formats) = try await track.load(.naturalSize, .preferredTransform, .estimatedDataRate, .nominalFrameRate, .formatDescriptions)
    let rotation = Int((atan2(Double(transform.b), Double(transform.a)) * 180 / .pi).rounded())
    guard let plan = captureUprightPlan(
      width: UInt32(size.width.rounded()), height: UInt32(size.height.rounded()), rotationDegrees: Int32(rotation),
      mirror: mirror, bitrate: UInt32(max(rate, 1)), frameRate: frameRate
    ) else { return }
    let hevc = formats.first.map { CMFormatDescriptionGetMediaSubType($0) == kCMVideoCodecType_HEVC } ?? false
    let audio = try await audioFacts(asset)
    let temp = url.deletingLastPathComponent().appendingPathComponent(".\(UUID().uuidString).upright")
    let entry = PrepareRunning()
    do {
      try await withTaskCancellationHandler {
        try await PrepareVideoWrite.run(source: url, output: temp, plan: plan, audio: audio, mirror: mirror, hevc: hevc, entry: entry, report: { _ in })
      } onCancel: { entry.cancel() }
      try Task.checkCancellation()
      _ = try FileManager.default.replaceItemAt(url, withItemAt: temp)
    } catch {
      try? FileManager.default.removeItem(at: temp)
      throw error
    }
    let milliseconds = (DispatchTime.now().uptimeNanoseconds - started) / 1_000_000
    MediaTrace.emit("camera", "take upright pass ms=\(milliseconds) mirror=\(mirror) rotation=\(rotation)")
  }

  /// The audio of the take as it was recorded: its own channels, rate and bitrate.
  private static func audioFacts(_ asset: AVURLAsset) async throws -> PrepareAudio {
    let fallback = PrepareAudio(bitrate: 128_000, channels: 2, sampleRate: 44_100)
    guard let track = try await asset.loadTracks(withMediaType: .audio).first else { return fallback }
    let (rate, formats) = try await track.load(.estimatedDataRate, .formatDescriptions)
    guard let format = formats.first, let description = CMAudioFormatDescriptionGetStreamBasicDescription(format)?.pointee else { return fallback }
    return PrepareAudio(
      bitrate: rate > 0 ? UInt32(rate) : fallback.bitrate,
      channels: max(description.mChannelsPerFrame, 1),
      sampleRate: description.mSampleRate > 0 ? UInt32(description.mSampleRate) : fallback.sampleRate
    )
  }
}
#endif
