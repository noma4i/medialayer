@preconcurrency import AVFoundation
import Foundation

extension MediaLayer {
  /// A video made to fit the limits: H.264 and AAC in mp4, upright, with even sides. A file that already fits comes back untouched, or copied without encoding when it carries location or personal tags. The result never carries them. `progress` gets 0 to 1 from the written samples.
  public func prepareVideo(
    _ source: some MediaSource,
    limits: VideoLimits = defaultVideoLimits(),
    progress: (@Sendable (Double) -> Void)? = nil
  ) async throws -> PreparedMedia {
    let file = try preparableFile(normalizeItem(item: source.item).url)
    let work = Task.detached { try await VideoPreparation.run(file, limits: limits, progress: progress) }
    return try await withTaskCancellationHandler { try await work.value } onCancel: { work.cancel() }
  }
}

enum VideoPreparation {
  static func run(_ file: URL, limits: VideoLimits, progress: (@Sendable (Double) -> Void)?) async throws -> PreparedMedia {
    let asset = AVURLAsset(url: file)
    guard let track = try await asset.loadTracks(withMediaType: .video).first else {
      throw MediaError.Fetch(detail: "\(file.path) has no video track")
    }
    let (size, transform, rate, dataRate, formats) = try await track.load(
      .naturalSize, .preferredTransform, .nominalFrameRate, .estimatedDataRate, .formatDescriptions
    )
    let duration = try await asset.load(.duration)
    let audioTrack = try await asset.loadTracks(withMediaType: .audio).first
    let audioFormats = try await audioTrack?.load(.formatDescriptions) ?? []
    let fits = ["mp4", "m4v"].contains(file.pathExtension.lowercased())
      && formats.allSatisfy { CMFormatDescriptionGetMediaSubType($0) == kCMVideoCodecType_H264 }
      && audioFormats.allSatisfy { CMFormatDescriptionGetMediaSubType($0) == kAudioFormatMPEG4AAC }
    let plan = try planVideo(
      source: VideoSource(
        width: UInt32(abs(size.width)), height: UInt32(abs(size.height)),
        rotationDegrees: Int32((atan2(transform.b, transform.a) * 180 / .pi).rounded()),
        frameRate: rate, bitrate: dataRate > 0 ? UInt32(dataRate) : nil, mp4H264Aac: fits
      ),
      limits: limits
    )
    if !plan.reencode {
      let keys = try await metadataKeys(asset, tracks: [track] + (audioTrack.map { [$0] } ?? []))
      guard !videoMetadataToStrip(keys: keys).isEmpty else { return PreparedMedia(url: file, isNew: false) }
    }
    try Task.checkCancellation()
    let output = preparedOutput(extension: "mp4")
    let job = try EncodeJob(
      asset: asset, track: track, audioTrack: audioTrack, naturalSize: size, transform: transform, duration: duration,
      encoding: plan.reencode ? (plan, limits) : nil, formats: (formats.first, audioFormats.first), output: output, progress: progress
    )
    do {
      try await withTaskCancellationHandler { try await job.run() } onCancel: { job.cancel() }
    } catch {
      try? FileManager.default.removeItem(at: output)
      throw error
    }
    progress?(1)
    return PreparedMedia(url: output, isNew: true)
  }
}

private func metadataKeys(_ asset: AVURLAsset, tracks: [AVAssetTrack]) async throws -> [String] {
  var items: [AVMetadataItem] = []
  for format in try await asset.load(.availableMetadataFormats) { items += try await asset.loadMetadata(for: format) }
  for track in tracks { items += try await track.load(.metadata) }
  return items.compactMap { ($0.identifier?.rawValue ?? ($0.key as? String)).map(decodedMetadataKey) }
}

/// AVFoundation escapes the bytes of a four-character key, so ©too arrives as `%A9too`, which is no UTF-8 and decodes to nothing or to U+FFFD with `removingPercentEncoding`.
func decodedMetadataKey(_ identifier: String) -> String {
  var bytes: [UInt8] = []
  var rest = Array(identifier.utf8)[...]
  while let byte = rest.popFirst() {
    let escaped = byte == UInt8(ascii: "%") ? rest.prefix(2).compactMap { Character(UnicodeScalar($0)).hexDigitValue } : []
    if escaped.count == 2 {
      bytes.append(UInt8(escaped[0] * 16 + escaped[1]))
      rest = rest.dropFirst(2)
    } else {
      bytes.append(byte)
    }
  }
  return String(bytes: bytes, encoding: .utf8) ?? String(String.UnicodeScalarView(bytes.map(UnicodeScalar.init)))
}

private final class EncodeJob: @unchecked Sendable {
  private let reader: AVAssetReader
  private let writer: AVAssetWriter
  private let videoOutput: AVAssetReaderOutput
  private let videoInput: AVAssetWriterInput
  private let audioOutput: AVAssetReaderOutput?
  private let audioInput: AVAssetWriterInput?
  private let duration: Double
  private let progress: (@Sendable (Double) -> Void)?
  private let queue = DispatchQueue(label: "MediaLayer.prepareVideo")
  private let lock = NSLock()
  private var cancelled = false
  private var gates: [PumpGate] = []

  init(
    asset: AVURLAsset, track: AVAssetTrack, audioTrack: AVAssetTrack?, naturalSize: CGSize, transform: CGAffineTransform,
    duration: CMTime, encoding: (plan: VideoPlan, limits: VideoLimits)?, formats: (video: CMFormatDescription?, audio: CMFormatDescription?),
    output: URL, progress: (@Sendable (Double) -> Void)?
  ) throws {
    reader = try AVAssetReader(asset: asset)
    writer = try AVAssetWriter(outputURL: output, fileType: .mp4)
    writer.shouldOptimizeForNetworkUse = true
    self.duration = duration.seconds
    self.progress = progress
    if let (plan, limits) = encoding {
      let composed = AVAssetReaderVideoCompositionOutput(videoTracks: [track], videoSettings: nil)
      composed.videoComposition = Self.composition(track: track, naturalSize: naturalSize, transform: transform, duration: duration, plan: plan)
      videoOutput = composed
      videoInput = AVAssetWriterInput(mediaType: .video, outputSettings: Self.videoSettings(plan))
      audioOutput = audioTrack.map { AVAssetReaderAudioMixOutput(audioTracks: [$0], audioSettings: Self.decodedAudio(limits)) }
      audioInput = audioTrack == nil ? nil : AVAssetWriterInput(mediaType: .audio, outputSettings: Self.encodedAudio(limits))
    } else {
      videoOutput = AVAssetReaderTrackOutput(track: track, outputSettings: nil)
      videoInput = AVAssetWriterInput(mediaType: .video, outputSettings: nil, sourceFormatHint: formats.video)
      videoInput.transform = transform
      audioOutput = audioTrack.map { AVAssetReaderTrackOutput(track: $0, outputSettings: nil) }
      audioInput = audioTrack == nil ? nil : AVAssetWriterInput(mediaType: .audio, outputSettings: nil, sourceFormatHint: formats.audio)
    }
    reader.add(videoOutput)
    videoInput.expectsMediaDataInRealTime = false
    writer.add(videoInput)
    if let audioOutput, let audioInput {
      reader.add(audioOutput)
      audioInput.expectsMediaDataInRealTime = false
      writer.add(audioInput)
    }
  }

  func cancel() {
    let waiting = lock.withLock {
      cancelled = true
      return gates
    }
    reader.cancelReading()
    writer.cancelWriting()
    waiting.forEach { $0.finish() }
  }

  func run() async throws {
    guard reader.startReading(), writer.startWriting() else {
      if lock.withLock({ cancelled }) { throw CancellationError() }
      throw MediaError.Unexpected(detail: "the video cannot be read or written")
    }
    writer.startSession(atSourceTime: .zero)
    async let video: Void = pump(input: videoInput, output: videoOutput, reportsProgress: true)
    async let audio: Void = pumpAudio()
    _ = try await (video, audio)
    if lock.withLock({ cancelled }) { throw CancellationError() }
    guard reader.status != .failed else {
      writer.cancelWriting()
      throw MediaError.Unexpected(detail: "the video cannot be read")
    }
    await writer.finishWriting()
    if lock.withLock({ cancelled }) { throw CancellationError() }
    guard writer.status == .completed else { throw MediaError.Unexpected(detail: "the video cannot be written") }
  }

  private func pumpAudio() async throws {
    guard let audioInput, let audioOutput else { return }
    try await pump(input: audioInput, output: audioOutput, reportsProgress: false)
  }

  private func pump(input: AVAssetWriterInput, output: AVAssetReaderOutput, reportsProgress: Bool) async throws {
    nonisolated(unsafe) let input = input
    nonisolated(unsafe) let output = output
    let gate = PumpGate()
    if lock.withLock({ gates.append(gate); return cancelled }) { gate.finish() }
    await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
      gate.wait(continuation)
      input.requestMediaDataWhenReady(on: queue) { [self] in
        while input.isReadyForMoreMediaData {
          guard let buffer = output.copyNextSampleBuffer(), input.append(buffer) else {
            if writer.status == .writing { input.markAsFinished() }
            gate.finish()
            return
          }
          if reportsProgress, duration > 0 {
            progress?(min(max(CMSampleBufferGetPresentationTimeStamp(buffer).seconds / duration, 0), 1))
          }
        }
      }
    }
  }

  private static func decodedAudio(_ limits: VideoLimits) -> [String: Any] {
    [
      AVFormatIDKey: kAudioFormatLinearPCM, AVSampleRateKey: Double(limits.audioSampleRate),
      AVNumberOfChannelsKey: Int(limits.audioChannels), AVLinearPCMBitDepthKey: 16,
      AVLinearPCMIsFloatKey: false, AVLinearPCMIsBigEndianKey: false, AVLinearPCMIsNonInterleaved: false
    ]
  }

  private static func encodedAudio(_ limits: VideoLimits) -> [String: Any] {
    [
      AVFormatIDKey: kAudioFormatMPEG4AAC, AVEncoderBitRateKey: Int(limits.audioBitrate),
      AVNumberOfChannelsKey: Int(limits.audioChannels), AVSampleRateKey: Double(limits.audioSampleRate)
    ]
  }

  private static func videoSettings(_ plan: VideoPlan) -> [String: Any] {
    [
      AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: Int(plan.width), AVVideoHeightKey: Int(plan.height),
      AVVideoScalingModeKey: AVVideoScalingModeResizeAspect,
      AVVideoCompressionPropertiesKey: [AVVideoAverageBitRateKey: Int(plan.bitrate), AVVideoProfileLevelKey: AVVideoProfileLevelH264HighAutoLevel]
    ]
  }

  private static func composition(
    track: AVAssetTrack, naturalSize: CGSize, transform: CGAffineTransform, duration: CMTime, plan: VideoPlan
  ) -> AVMutableVideoComposition {
    let bounds = CGRect(origin: .zero, size: naturalSize).applying(transform)
    let scale = CGAffineTransform(scaleX: CGFloat(plan.width) / bounds.width, y: CGFloat(plan.height) / bounds.height)
    let upright = transform.concatenating(CGAffineTransform(translationX: -bounds.minX, y: -bounds.minY)).concatenating(scale)
    let layer = AVMutableVideoCompositionLayerInstruction(assetTrack: track)
    layer.setTransform(upright, at: .zero)
    let instruction = AVMutableVideoCompositionInstruction()
    instruction.timeRange = CMTimeRange(start: .zero, duration: duration)
    instruction.layerInstructions = [layer]
    let composition = AVMutableVideoComposition()
    composition.renderSize = CGSize(width: Int(plan.width), height: Int(plan.height))
    composition.frameDuration = CMTime(value: 1, timescale: CMTimeScale(plan.frameRate))
    composition.instructions = [instruction]
    return composition
  }
}

private final class PumpGate: @unchecked Sendable {
  private let lock = NSLock()
  private var continuation: CheckedContinuation<Void, Never>?
  private var finished = false

  func wait(_ continuation: CheckedContinuation<Void, Never>) {
    let ready = lock.withLock {
      if !finished { self.continuation = continuation }
      return finished
    }
    if ready { continuation.resume() }
  }

  func finish() {
    let waiting = lock.withLock {
      finished = true
      defer { continuation = nil }
      return continuation
    }
    waiting?.resume()
  }
}
