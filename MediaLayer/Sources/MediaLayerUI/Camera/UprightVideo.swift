@preconcurrency import AVFoundation
import Foundation
import MediaLayer

func uprightVideo(at url: URL) async throws {
  let task = Task.detached {
    try Task.checkCancellation()
    let asset = AVURLAsset(url: url)
    guard let track = try await asset.loadTracks(withMediaType: .video).first else { throw UprightVideoError.invalidVideo }
    let (transform, size, bitrate, descriptions) = try await track.load(.preferredTransform, .naturalSize, .estimatedDataRate, .formatDescriptions)
    if MediaTrace.isOn {
      let audioTracks = (try? await asset.loadTracks(withMediaType: .audio).count) ?? 0
      MediaTrace.emit("camera", "clip audioTracks=\(audioTracks) rewrite=\(transform != .identity)")
    }
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
    defer { NativeFailures.removeFile(temporary) }
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
    let gates = pairs.map { _ in UprightGate() }
    await withTaskCancellationHandler {
      await withTaskGroup(of: Void.self) { group in
        for (index, (input, output)) in pairs.enumerated() {
          let gate = gates[index]
          group.addTask { await writeSamples(input: input, output: output, gate: gate) }
        }
      }
    } onCancel: {
      reader.cancelReading()
      writer.cancelWriting()
      gates.forEach { $0.finish() }
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

/// One pump waits on its gate: the end of its samples or the cancel of the whole job opens it, so a cancel never waits for a callback of the platform.
private final class UprightGate: @unchecked Sendable {
  private let lock = NSLock()
  private var continuation: CheckedContinuation<Void, Never>?
  private var finished = false

  var isFinished: Bool { lock.withLock { finished } }

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

private func writeSamples(input: AVAssetWriterInput, output: AVAssetReaderOutput, gate: UprightGate) async {
  nonisolated(unsafe) let input = input
  nonisolated(unsafe) let output = output
  await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
    gate.wait(continuation)
    let queue = DispatchQueue(label: "MediaLayer.uprightVideo.samples.\(UUID().uuidString)")
    input.requestMediaDataWhenReady(on: queue) {
      while input.isReadyForMoreMediaData {
        if gate.isFinished { return }
        guard let sample = output.copyNextSampleBuffer(), input.append(sample) else {
          input.markAsFinished()
          gate.finish()
          return
        }
      }
    }
  }
}
