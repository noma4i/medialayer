@preconcurrency import AVFoundation
import Foundation

func uprightVideo(at url: URL) async throws {
  let task = Task.detached {
    try Task.checkCancellation()
    let asset = AVURLAsset(url: url)
    guard let track = try await asset.loadTracks(withMediaType: .video).first else { throw UprightVideoError.invalidVideo }
    let (transform, size, bitrate, descriptions) = try await track.load(.preferredTransform, .naturalSize, .estimatedDataRate, .formatDescriptions)
    guard transform != .identity else { return }
    guard let format = descriptions.first else { throw UprightVideoError.invalidVideo }
    let codec: AVVideoCodecType
    switch CMFormatDescriptionGetMediaSubType(format) {
    case kCMVideoCodecType_H264: codec = .h264
    case kCMVideoCodecType_HEVC: codec = .hevc
    default: throw UprightVideoError.invalidVideo
    }
    let bounds = CGRect(origin: .zero, size: size).applying(transform).standardized
    let width = Int(ceil(bounds.width / 2)) * 2
    let height = Int(ceil(bounds.height / 2)) * 2
    guard width > 0, height > 0 else { throw UprightVideoError.invalidVideo }
    let temporary = url.deletingLastPathComponent().appendingPathComponent(".upright-\(UUID().uuidString).\(url.pathExtension)")
    defer { try? FileManager.default.removeItem(at: temporary) }
    let reader = try AVAssetReader(asset: asset)
    let fileType: AVFileType = url.pathExtension.lowercased() == "mov" ? .mov : .mp4
    let writer = try AVAssetWriter(outputURL: temporary, fileType: fileType)
    let composition = AVMutableVideoComposition(propertiesOf: asset)
    composition.renderSize = CGSize(width: width, height: height)
    let videoOutput = AVAssetReaderVideoCompositionOutput(videoTracks: [track], videoSettings: [kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA])
    videoOutput.videoComposition = composition
    let videoInput = AVAssetWriterInput(mediaType: .video, outputSettings: [
      AVVideoCodecKey: codec,
      AVVideoWidthKey: width,
      AVVideoHeightKey: height,
      AVVideoCompressionPropertiesKey: [AVVideoAverageBitRateKey: Int(bitrate)]
    ])
    guard reader.canAdd(videoOutput), writer.canAdd(videoInput) else { throw UprightVideoError.invalidVideo }
    reader.add(videoOutput)
    writer.add(videoInput)
    var pairs = [(videoInput, videoOutput as AVAssetReaderOutput)]
    if let audio = try await asset.loadTracks(withMediaType: .audio).first {
      guard let audioFormat = try await audio.load(.formatDescriptions).first else { throw UprightVideoError.invalidVideo }
      let output = AVAssetReaderTrackOutput(track: audio, outputSettings: nil)
      let input = AVAssetWriterInput(mediaType: .audio, outputSettings: nil, sourceFormatHint: audioFormat)
      guard reader.canAdd(output), writer.canAdd(input) else { throw UprightVideoError.invalidVideo }
      reader.add(output)
      writer.add(input)
      pairs.append((input, output))
    }
    guard reader.startReading(), writer.startWriting() else { throw reader.error ?? writer.error ?? UprightVideoError.invalidVideo }
    writer.startSession(atSourceTime: .zero)
    await withTaskCancellationHandler {
      await withTaskGroup(of: Void.self) { group in
        for (input, output) in pairs {
          group.addTask { await writeSamples(input: input, output: output) }
        }
      }
    } onCancel: {
      reader.cancelReading()
      writer.cancelWriting()
    }
    try Task.checkCancellation()
    guard reader.status == .completed, writer.status == .writing else {
      writer.cancelWriting()
      throw reader.error ?? writer.error ?? UprightVideoError.invalidVideo
    }
    await writer.finishWriting()
    guard writer.status == .completed else { throw writer.error ?? UprightVideoError.invalidVideo }
    try Task.checkCancellation()
    _ = try FileManager.default.replaceItemAt(url, withItemAt: temporary)
  }
  try await withTaskCancellationHandler {
    try await task.value
  } onCancel: {
    task.cancel()
  }
}

private enum UprightVideoError: Error {
  case invalidVideo
}

private func writeSamples(input: AVAssetWriterInput, output: AVAssetReaderOutput) async {
  nonisolated(unsafe) let input = input
  nonisolated(unsafe) let output = output
  await withCheckedContinuation { continuation in
    let queue = DispatchQueue(label: "MediaLayer.uprightVideo.samples.\(UUID().uuidString)")
    input.requestMediaDataWhenReady(on: queue) {
      while input.isReadyForMoreMediaData {
        guard let sample = output.copyNextSampleBuffer(), input.append(sample) else {
          input.markAsFinished()
          continuation.resume()
          return
        }
      }
    }
  }
}
