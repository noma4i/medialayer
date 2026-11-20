@preconcurrency import AVFoundation
import Foundation

final class PrepareSerial<Value>: @unchecked Sendable {
  private let queue = DispatchQueue(label: "medialayer.prepare.state")
  private var value: Value

  init(_ value: Value) { self.value = value }

  func update<Result>(_ body: (inout Value) -> Result) -> Result { queue.sync { body(&value) } }
}

struct PrepareAudio {
  let bitrate: UInt32
  let channels: UInt32
  let sampleRate: UInt32
}

struct PrepareEncoding {
  let plan: VideoPlan
  let audio: PrepareAudio
  let mirror: Bool
}

final class PrepareRunning: @unchecked Sendable {
  private struct State {
    var task: Task<MediaPrepareOutcome, Never>?
    var job: PrepareEncodeJob?
    var cancelled = false
  }

  private let state = PrepareSerial(State())

  func hold(_ task: Task<MediaPrepareOutcome, Never>) {
    let stop = state.update { state -> Bool in
      state.task = task
      return state.cancelled
    }
    if stop { task.cancel() }
  }

  fileprivate func attach(_ job: PrepareEncodeJob) {
    let stop = state.update { state -> Bool in
      state.job = job
      return state.cancelled
    }
    if stop { job.cancel() }
  }

  func cancel() {
    let (task, job) = state.update { state -> (Task<MediaPrepareOutcome, Never>?, PrepareEncodeJob?) in
      state.cancelled = true
      return (state.task, state.job)
    }
    task?.cancel()
    job?.cancel()
  }
}

enum PrepareVideoWrite {
  static func run(
    source: URL, output: URL, plan: VideoPlan, audio: PrepareAudio, mirror: Bool, entry: PrepareRunning, report: @escaping @Sendable (Double) -> Void
  ) async throws {
    let asset = PrepareContainer.asset(source)
    guard let track = try await asset.loadTracks(withMediaType: .video).first else {
      throw PrepareFailure(message: "\(source.path) has no video track")
    }
    let (size, transform, formats) = try await track.load(.naturalSize, .preferredTransform, .formatDescriptions)
    let duration = try await asset.load(.duration)
    let audioTrack = try await asset.loadTracks(withMediaType: .audio).first
    let audioFormats = try await audioTrack?.load(.formatDescriptions) ?? []
    try Task.checkCancellation()
    let encode = try PrepareEncodeJob(
      asset: asset, track: track, audioTrack: audioTrack, naturalSize: size, transform: transform, duration: duration,
      encoding: plan.reencode || mirror ? PrepareEncoding(plan: plan, audio: audio, mirror: mirror) : nil,
      formats: (formats.first, audioFormats.first), output: output, report: report
    )
    entry.attach(encode)
    try await encode.run()
    report(1)
  }
}

final class PrepareEncodeJob: @unchecked Sendable {
  private struct State {
    var cancelled = false
    var gates: [PreparePumpGate] = []
  }

  private let reader: AVAssetReader
  private let writer: AVAssetWriter
  private let videoOutput: AVAssetReaderOutput
  private let videoInput: AVAssetWriterInput
  private let audioOutput: AVAssetReaderOutput?
  private let audioInput: AVAssetWriterInput?
  private let duration: Double
  private let report: @Sendable (Double) -> Void
  private let queue = DispatchQueue(label: "medialayer.prepare.video")
  private let state = PrepareSerial(State())
  private let stopped = PreparePumpGate()

  init(
    asset: AVURLAsset, track: AVAssetTrack, audioTrack: AVAssetTrack?, naturalSize: CGSize, transform: CGAffineTransform,
    duration: CMTime, encoding: PreparePrepareEncoding?, formats: (video: CMFormatDescription?, audio: CMFormatDescription?),
    output: URL, report: @escaping @Sendable (Double) -> Void
  ) throws {
    reader = try AVAssetReader(asset: asset)
    writer = try AVAssetWriter(outputURL: output, fileType: .mp4)
    writer.shouldOptimizeForNetworkUse = true
    self.duration = duration.seconds
    self.report = report
    if let encoding {
      let composed = AVAssetReaderVideoCompositionOutput(videoTracks: [track], videoSettings: nil)
      composed.videoComposition = Self.composition(track: track, naturalSize: naturalSize, transform: transform, duration: duration, encoding: encoding)
      videoOutput = composed
      videoInput = AVAssetWriterInput(mediaType: .video, outputSettings: Self.videoSettings(encoding.plan))
      audioOutput = audioTrack.map { AVAssetReaderAudioMixOutput(audioTracks: [$0], audioSettings: Self.decodedAudio(encoding.audio)) }
      audioInput = audioTrack == nil ? nil : AVAssetWriterInput(mediaType: .audio, outputSettings: Self.encodedAudio(encoding.audio))
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
    let waiting = state.update { state -> [PreparePumpGate] in
      state.cancelled = true
      return state.gates
    }
    waiting.forEach { $0.finish() }
    queue.async { [self] in
      reader.cancelReading()
      writer.cancelWriting()
      stopped.finish()
    }
  }

  func run() async throws {
    guard reader.startReading(), writer.startWriting() else {
      if isCancelled { throw CancellationError() }
      throw PrepareFailure(message: "the video cannot be read or written")
    }
    writer.startSession(atSourceTime: .zero)
    async let video: Void = pump(input: videoInput, output: videoOutput, reportsProgress: true)
    async let audio: Void = pumpAudio()
    _ = try await (video, audio)
    if isCancelled {
      await withCheckedContinuation { stopped.wait($0) }
      throw CancellationError()
    }
    guard reader.status != .failed else {
      writer.cancelWriting()
      throw PrepareFailure(message: "the video cannot be read")
    }
    await writer.finishWriting()
    if isCancelled {
      await withCheckedContinuation { stopped.wait($0) }
      throw CancellationError()
    }
    guard writer.status == .completed else { throw PrepareFailure(message: "the video cannot be written") }
  }

  private var isCancelled: Bool { state.update { $0.cancelled } }

  private func pumpAudio() async throws {
    guard let audioInput, let audioOutput else { return }
    try await pump(input: audioInput, output: audioOutput, reportsProgress: false)
  }

  private func pump(input: AVAssetWriterInput, output: AVAssetReaderOutput, reportsProgress: Bool) async throws {
    let gate = PreparePumpGate()
    if state.update({ state -> Bool in
      state.gates.append(gate)
      return state.cancelled
    }) { gate.finish() }
    await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
      gate.wait(continuation)
      input.requestMediaDataWhenReady(on: queue) { [self] in
        while input.isReadyForMoreMediaData {
          if isCancelled {
            gate.finish()
            return
          }
          guard let buffer = output.copyNextSampleBuffer(), input.append(buffer) else {
            if writer.status == .writing { input.markAsFinished() }
            gate.finish()
            return
          }
          if reportsProgress, duration > 0 {
            report(min(max(CMSampleBufferGetPresentationTimeStamp(buffer).seconds / duration, 0), 1))
          }
        }
      }
    }
  }

  private static func decodedAudio(_ audio: PrepareAudio) -> [String: Any] {
    [
      AVFormatIDKey: kAudioFormatLinearPCM, AVSampleRateKey: Double(audio.sampleRate),
      AVNumberOfChannelsKey: Int(audio.channels), AVLinearPCMBitDepthKey: 16,
      AVLinearPCMIsFloatKey: false, AVLinearPCMIsBigEndianKey: false, AVLinearPCMIsNonInterleaved: false
    ]
  }

  private static func encodedAudio(_ audio: PrepareAudio) -> [String: Any] {
    [
      AVFormatIDKey: kAudioFormatMPEG4AAC, AVEncoderBitRateKey: Int(audio.bitrate),
      AVNumberOfChannelsKey: Int(audio.channels), AVSampleRateKey: Double(audio.sampleRate)
    ]
  }

  private static func videoSettings(_ plan: VideoPlan) -> [String: Any] {
    [
      AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: Int(plan.width), AVVideoHeightKey: Int(plan.height),
      AVVideoScalingModeKey: AVVideoScalingModeResizeAspect,
      AVVideoColorPropertiesKey: [
        AVVideoColorPrimariesKey: AVVideoColorPrimaries_ITU_R_709_2,
        AVVideoTransferFunctionKey: AVVideoTransferFunction_ITU_R_709_2,
        AVVideoYCbCrMatrixKey: AVVideoYCbCrMatrix_ITU_R_709_2
      ],
      AVVideoCompressionPropertiesKey: [
        AVVideoAverageBitRateKey: Int(plan.bitrate), AVVideoProfileLevelKey: AVVideoProfileLevelH264HighAutoLevel,
        AVVideoExpectedSourceFrameRateKey: Int(plan.frameRate)
      ]
    ]
  }

  private static func composition(
    track: AVAssetTrack, naturalSize: CGSize, transform: CGAffineTransform, duration: CMTime, encoding: PrepareEncoding
  ) -> AVMutableVideoComposition {
    let plan = encoding.plan
    let bounds = CGRect(origin: .zero, size: naturalSize).applying(transform)
    let scale = CGAffineTransform(scaleX: CGFloat(plan.width) / bounds.width, y: CGFloat(plan.height) / bounds.height)
    var upright = transform.concatenating(CGAffineTransform(translationX: -bounds.minX, y: -bounds.minY)).concatenating(scale)
    if encoding.mirror { upright = upright.concatenating(CGAffineTransform(a: -1, b: 0, c: 0, d: 1, tx: CGFloat(plan.width), ty: 0)) }
    let layer = AVMutableVideoCompositionLayerInstruction(assetTrack: track)
    layer.setTransform(upright, at: .zero)
    let instruction = AVMutableVideoCompositionInstruction()
    instruction.timeRange = CMTimeRange(start: .zero, duration: duration)
    instruction.layerInstructions = [layer]
    let composition = AVMutableVideoComposition()
    composition.renderSize = CGSize(width: Int(plan.width), height: Int(plan.height))
    composition.frameDuration = CMTime(value: 1, timescale: CMTimeScale(plan.frameRate))
    composition.colorPrimaries = AVVideoColorPrimaries_ITU_R_709_2
    composition.colorTransferFunction = AVVideoTransferFunction_ITU_R_709_2
    composition.colorYCbCrMatrix = AVVideoYCbCrMatrix_ITU_R_709_2
    composition.instructions = [instruction]
    return composition
  }
}

private final class PreparePumpGate: @unchecked Sendable {
  private struct State {
    var continuation: CheckedContinuation<Void, Never>?
    var finished = false
  }

  private let state = PrepareSerial(State())

  func wait(_ continuation: CheckedContinuation<Void, Never>) {
    let ready = state.update { state -> Bool in
      if !state.finished { state.continuation = continuation }
      return state.finished
    }
    if ready { continuation.resume() }
  }

  func finish() {
    let waiting = state.update { state -> CheckedContinuation<Void, Never>? in
      state.finished = true
      defer { state.continuation = nil }
      return state.continuation
    }
    waiting?.resume()
  }
}


struct PrepareFailure: LocalizedError {
  let message: String
  var errorDescription: String? { message }
}
