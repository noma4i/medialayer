@preconcurrency import AVFoundation
import CoreGraphics
import Foundation
import ImageIO
import MediaLayer
import Testing
import UniformTypeIdentifiers
@testable import MediaLayerUI

struct FixtureClip {
  var width: Int
  var height: Int
  var codec = AVVideoCodecType.h264
  var audio = false
  var turned = false
  var mirrored = false
  var metadata = false
  var hdr = false
  var frames = 30
}

private func writeClip(_ url: URL, width: Int, height: Int, frames: Int) async throws {
  try await writeClip(FixtureClip(width: width, height: height, frames: frames), to: url)
}

/// A real clip written by AVAssetWriter: left half white, right half black, optional AAC track, private metadata, 90 degree turn or HDR.
private func writeClip(_ clip: FixtureClip, to url: URL) async throws {
  let writer = try AVAssetWriter(outputURL: url, fileType: .mp4)
  if clip.metadata {
    let location = AVMutableMetadataItem()
    location.keySpace = .quickTimeMetadata
    location.key = AVMetadataKey.quickTimeMetadataKeyLocationISO6709 as NSString
    location.identifier = .quickTimeMetadataLocationISO6709
    location.value = "+48.8583+002.2945/" as NSString
    location.dataType = "com.apple.metadata.datatype.UTF-8"
    let title = AVMutableMetadataItem()
    title.identifier = .iTunesMetadataSongName
    title.value = "Private title" as NSString
    title.dataType = "com.apple.metadata.datatype.UTF-8"
    writer.metadata = [location, title]
  }
  var settings: [String: Any] = [AVVideoCodecKey: clip.codec, AVVideoWidthKey: clip.width, AVVideoHeightKey: clip.height]
  if clip.hdr {
    settings[AVVideoColorPropertiesKey] = [
      AVVideoColorPrimariesKey: AVVideoColorPrimaries_ITU_R_2020,
      AVVideoTransferFunctionKey: AVVideoTransferFunction_ITU_R_2100_HLG,
      AVVideoYCbCrMatrixKey: AVVideoYCbCrMatrix_ITU_R_2020
    ]
    settings[AVVideoCompressionPropertiesKey] = [AVVideoProfileLevelKey: "HEVC_Main10_AutoLevel"]
  }
  let video = AVAssetWriterInput(mediaType: .video, outputSettings: settings)
  if clip.turned { video.transform = CGAffineTransform(rotationAngle: .pi / 2) }
  if clip.mirrored { video.transform = CGAffineTransform(a: -1, b: 0, c: 0, d: 1, tx: CGFloat(clip.width), ty: 0) }
  writer.add(video)
  let format = clip.hdr ? kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange : kCVPixelFormatType_32BGRA
  let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: video, sourcePixelBufferAttributes: nil)
  var audio: AVAssetWriterInput?
  var audioFormat: CMAudioFormatDescription?
  if clip.audio {
    var description = AudioStreamBasicDescription(
      mSampleRate: 44100, mFormatID: kAudioFormatLinearPCM, mFormatFlags: kAudioFormatFlagIsSignedInteger | kAudioFormatFlagIsPacked,
      mBytesPerPacket: 2, mFramesPerPacket: 1, mBytesPerFrame: 2, mChannelsPerFrame: 1, mBitsPerChannel: 16, mReserved: 0
    )
    CMAudioFormatDescriptionCreate(allocator: nil, asbd: &description, layoutSize: 0, layout: nil, magicCookieSize: 0, magicCookie: nil, extensions: nil, formatDescriptionOut: &audioFormat)
    let input = AVAssetWriterInput(
      mediaType: .audio, outputSettings: [AVFormatIDKey: kAudioFormatMPEG4AAC, AVNumberOfChannelsKey: 1, AVSampleRateKey: 44100, AVEncoderBitRateKey: 64000]
    )
    writer.add(input)
    audio = input
  }
  try #require(writer.startWriting())
  writer.startSession(atSourceTime: .zero)
  nonisolated(unsafe) let adaptorInFeed = adaptor
  async let videoFed: Void = feed(video, count: clip.frames) { index in
    var buffer: CVPixelBuffer?
    CVPixelBufferCreate(nil, clip.width, clip.height, format, nil, &buffer)
    guard let buffer else { return }
    CVPixelBufferLockBaseAddress(buffer, [])
    if !clip.hdr, let base = CVPixelBufferGetBaseAddress(buffer) {
      let stride = CVPixelBufferGetBytesPerRow(buffer)
      for row in 0..<clip.height {
        let line = base.advanced(by: row * stride).assumingMemoryBound(to: UInt8.self)
        for column in 0..<clip.width {
          let level: UInt8 = column < clip.width / 2 ? 255 : 0
          line[column * 4] = level
          line[column * 4 + 1] = level
          line[column * 4 + 2] = level
          line[column * 4 + 3] = 255
        }
      }
    }
    CVPixelBufferUnlockBaseAddress(buffer, [])
    adaptorInFeed.append(buffer, withPresentationTime: CMTime(value: CMTimeValue(index), timescale: 30))
  }
  // Both inputs are fed at once: the writer interleaves them, so one input waits for the other when fed in turn.
  async let audioFed: Void = feedAudio(audio, format: audioFormat)
  await videoFed
  await audioFed
  await writer.finishWriting()
  try #require(writer.status == .completed, "the fixture clip cannot be written: \(String(describing: writer.error))")
}

private func feedAudio(_ audio: AVAssetWriterInput?, format audioFormat: CMAudioFormatDescription?) async {
  if let audio, let audioFormat {
    let samples = 44100
    var block: CMBlockBuffer?
    CMBlockBufferCreateWithMemoryBlock(
      allocator: nil, memoryBlock: nil, blockLength: samples * 2, blockAllocator: nil, customBlockSource: nil,
      offsetToData: 0, dataLength: samples * 2, flags: kCMBlockBufferAssureMemoryNowFlag, blockBufferOut: &block
    )
    if let block {
      CMBlockBufferFillDataBytes(with: 0, blockBuffer: block, offsetIntoDestination: 0, dataLength: samples * 2)
      var sample: CMSampleBuffer?
      CMAudioSampleBufferCreateReadyWithPacketDescriptions(
        allocator: nil, dataBuffer: block, formatDescription: audioFormat, sampleCount: samples,
        presentationTimeStamp: .zero, packetDescriptions: nil, sampleBufferOut: &sample
      )
      nonisolated(unsafe) let held = sample
      await feed(audio, count: 1) { _ in if let held { audio.append(held) } }
    }
  }
}

private func feed(_ input: AVAssetWriterInput, count: Int, append: @escaping (Int) -> Void) async {
  nonisolated(unsafe) let append = append
  nonisolated(unsafe) let input = input
  await withCheckedContinuation { (done: CheckedContinuation<Void, Never>) in
    nonisolated(unsafe) var next = 0
    nonisolated(unsafe) var finished = false
    input.requestMediaDataWhenReady(on: DispatchQueue(label: "preparer.fixture.feed")) {
      guard !finished else { return }
      for index in next..<count {
        guard input.isReadyForMoreMediaData else {
          next = index
          return
        }
        append(index)
      }
      next = count
      input.markAsFinished()
      finished = true
      done.resume()
    }
  }
}

private func writePicture(_ url: URL, type: UTType = .jpeg, width: Int, height: Int, orientation: UInt32, alpha: Bool = false, tagged: Bool = false) throws {
  let context = try #require(CGContext(
    data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(),
    bitmapInfo: (alpha ? CGImageAlphaInfo.premultipliedLast : CGImageAlphaInfo.noneSkipLast).rawValue
  ))
  context.setFillColor(CGColor(red: 1, green: 0, blue: 0, alpha: 1))
  context.fill(CGRect(x: 0, y: 0, width: width / 2, height: height))
  context.setFillColor(CGColor(red: 0, green: 0, blue: 1, alpha: alpha ? 0.5 : 1))
  context.fill(CGRect(x: width / 2, y: 0, width: width - width / 2, height: height))
  var properties: [CFString: Any] = [kCGImagePropertyOrientation: orientation]
  if tagged {
    properties[kCGImagePropertyGPSDictionary] = [
      kCGImagePropertyGPSLatitude: 37.33, kCGImagePropertyGPSLatitudeRef: "N", kCGImagePropertyGPSLongitude: 122.03, kCGImagePropertyGPSLongitudeRef: "W"
    ]
    properties[kCGImagePropertyTIFFDictionary] = [kCGImagePropertyTIFFMake: "Acme", kCGImagePropertyTIFFModel: "Cam 1"]
  }
  let destination = try #require(CGImageDestinationCreateWithURL(url as CFURL, type.identifier as CFString, 1, nil), "\(type.identifier) cannot be written on this runtime")
  CGImageDestinationAddImage(destination, try #require(context.makeImage()), properties as CFDictionary)
  try #require(CGImageDestinationFinalize(destination), "\(type.identifier) cannot be encoded on this runtime")
}

private func scratch() throws -> URL {
  let directory = FileManager.default.temporaryDirectory.appending(path: UUID().uuidString)
  try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
  return directory
}

private func videoDescriptor(limits: VideoLimits? = nil, mirror: Bool = false) throws -> Data {
  try mediaPrepareDescriptor(intent: .video(limits: limits, mirror: mirror, onCannotDecode: .fail))
}

private let plainEdit = PhotoEdit(maxSize: nil, enlarge: false, crop: nil, outputSize: nil, mirror: false, rotationDegrees: 0)

@Suite(.serialized)
struct MediaPreparerTests {
  @Test(.timeLimit(.minutes(2)))
  func aSourceThatFitsReturnsUnchangedAndWritesNothing() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "source.mp4")
    try await writeClip(source, width: 64, height: 32, frames: 30)
    let destination = directory.appending(path: "result.mp4")
    let outcome = await MediaPreparer().run(job: MediaPrepareJob(attemptId: 1, source: source.path, destination: destination.path, descriptor: try videoDescriptor())) { _ in }
    guard case .unchanged = outcome else {
      Issue.record("the source fits, got \(outcome)")
      return
    }
    #expect(!FileManager.default.fileExists(atPath: destination.path))
    #expect(try FileManager.default.contentsOfDirectory(atPath: directory.path).sorted() == ["source.mp4"])
  }

  @Test(.timeLimit(.minutes(2)))
  func aPhotoIsWrittenUprightByTheDescriptor() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "photo.jpg")
    try writePicture(source, width: 40, height: 20, orientation: 6)
    let destination = directory.appending(path: "result.jpg")
    let descriptor = try mediaPrepareDescriptor(intent: .image(edit: plainEdit, quality: nil, format: .jpeg))
    let progress = PrepareProgressLog()
    let outcome = await MediaPreparer().run(job: MediaPrepareJob(attemptId: 2, source: source.path, destination: destination.path, descriptor: descriptor)) { progress.add($0) }
    guard case let .done(_, mime) = outcome else {
      Issue.record("the photo is prepared, got \(outcome)")
      return
    }
    #expect(mime == "image/jpeg")
    let image = try #require(CGImageSourceCreateWithURL(destination as CFURL, nil))
    let properties = try #require(CGImageSourceCopyPropertiesAtIndex(image, 0, nil) as? [CFString: Any])
    #expect(properties[kCGImagePropertyPixelWidth] as? Int == 20 && properties[kCGImagePropertyPixelHeight] as? Int == 40)
    #expect(progress.values.last == 1 && progress.values == progress.values.sorted())
    #expect(try FileManager.default.contentsOfDirectory(atPath: directory.path).sorted() == ["photo.jpg", "result.jpg"])
  }

  @Test(.timeLimit(.minutes(2)))
  func aVideoAboveTheLimitsIsWrittenSmallerWithRisingProgress() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "source.mp4")
    try await writeClip(source, width: 640, height: 360, frames: 30)
    let destination = directory.appending(path: "result.mp4")
    var limits = defaultVideoLimits()
    limits.maxLongSide = 320
    let progress = PrepareProgressLog()
    let outcome = await MediaPreparer().run(job: MediaPrepareJob(attemptId: 3, source: source.path, destination: destination.path, descriptor: try videoDescriptor(limits: limits))) { progress.add($0) }
    guard case let .done(_, mime) = outcome else {
      Issue.record("the video is prepared, got \(outcome)")
      return
    }
    #expect(mime == "video/mp4")
    let track = try #require(try await AVURLAsset(url: destination).loadTracks(withMediaType: .video).first)
    #expect(try await track.load(.naturalSize).width == 320)
    #expect(progress.values.last == 1 && progress.values == progress.values.sorted())
  }

  @Test(.timeLimit(.minutes(2)))
  func cancelReturnsCancelledAndLeavesNoFile() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "long.mp4")
    try await writeClip(source, width: 640, height: 360, frames: 240)
    let destination = directory.appending(path: "result.mp4")
    var limits = defaultVideoLimits()
    limits.maxLongSide = 320
    let preparer = MediaPreparer()
    let first = AsyncStream<Double>.makeStream()
    let job = MediaPrepareJob(attemptId: 7, source: source.path, destination: destination.path, descriptor: try videoDescriptor(limits: limits))
    let work = Task { await preparer.run(job: job) { first.continuation.yield($0) } }
    var values = first.stream.makeAsyncIterator()
    _ = await values.next()
    preparer.cancel(attemptId: 7)
    let outcome = await work.value
    #expect(outcome == .cancelled)
    #expect(try FileManager.default.contentsOfDirectory(atPath: directory.path).sorted() == ["long.mp4"])
  }

  @Test(.timeLimit(.minutes(1)))
  func anUnsupportedDescriptorFailsAndWritesNothing() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "source.mp4")
    try await writeClip(source, width: 64, height: 32, frames: 3)
    let destination = directory.appending(path: "result.mp4")
    let outcome = await MediaPreparer().run(job: MediaPrepareJob(attemptId: 4, source: source.path, destination: destination.path, descriptor: Data([9, 9, 9]))) { _ in }
    guard case let .failed(message) = outcome else {
      Issue.record("the descriptor is unsupported, got \(outcome)")
      return
    }
    #expect(!message.isEmpty)
    #expect(!FileManager.default.fileExists(atPath: destination.path))
  }

  @Test(.timeLimit(.minutes(1)))
  func probeGivesTheUprightSizeAndMimeOfThePlannedResult() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "photo.jpg")
    try writePicture(source, width: 40, height: 20, orientation: 6)
    guard case let .file(width, height, mime) = await MediaPreparer().probe(path: source.path, descriptor: nil, mime: nil) else {
      Issue.record("the probe reads the photo")
      return
    }
    #expect(width == 20 && height == 40 && mime == "image/jpeg")
    guard case .failed = await MediaPreparer().probe(path: directory.appending(path: "none").path, descriptor: nil, mime: nil) else {
      Issue.record("a missing file fails the probe")
      return
    }
  }
}

private final class PrepareProgressLog: @unchecked Sendable {
  private let lock = NSLock()
  private var seen: [Double] = []
  func add(_ value: Double) { lock.withLock { seen.append(value) } }
  var values: [Double] { lock.withLock { seen } }
}

// MARK: - Facts read back from the written result

private struct VideoFacts {
  var width: Int
  var height: Int
  var codec: FourCharCode
  var frames: Int
  var audio: Bool
  var audioCodec: FourCharCode?
  var primaries: String?
  var metadata: [String]
}

private func videoFacts(_ url: URL) async throws -> VideoFacts {
  let asset = AVURLAsset(url: url)
  let track = try #require(try await asset.loadTracks(withMediaType: .video).first)
  let (size, transform, formats) = try await track.load(.naturalSize, .preferredTransform, .formatDescriptions)
  let upright = size.applying(transform)
  let reader = try AVAssetReader(asset: asset)
  let output = AVAssetReaderTrackOutput(track: track, outputSettings: nil)
  reader.add(output)
  reader.startReading()
  let frames = sequence(state: output) { $0.copyNextSampleBuffer() }.reduce(0) { $0 + CMSampleBufferGetNumSamples($1) }
  let extensions = formats.first.flatMap { CMFormatDescriptionGetExtensions($0) as? [String: Any] }
  let audioTrack = try await asset.loadTracks(withMediaType: .audio).first
  let audioFormats = try await audioTrack?.load(.formatDescriptions) ?? []
  var items: [AVMetadataItem] = []
  for format in try await asset.load(.availableMetadataFormats) { items += try await asset.loadMetadata(for: format) }
  items += try await track.load(.metadata)
  return VideoFacts(
    width: Int(abs(upright.width).rounded()), height: Int(abs(upright.height).rounded()),
    codec: CMFormatDescriptionGetMediaSubType(try #require(formats.first)), frames: frames,
    audio: audioTrack != nil, audioCodec: audioFormats.first.map { CMFormatDescriptionGetMediaSubType($0) },
    primaries: extensions?[kCMFormatDescriptionExtension_ColorPrimaries as String] as? String,
    metadata: items.compactMap { $0.identifier?.rawValue ?? $0.commonKey?.rawValue }
  )
}

/// The brightness 0...1 of the left or right edge of the first frame.
private func luminance(_ url: URL, left: Bool) async throws -> Double {
  let generator = AVAssetImageGenerator(asset: AVURLAsset(url: url))
  let image = try generator.copyCGImage(at: .zero, actualTime: nil)
  return try level(image, x: left ? 2 : image.width - 3, y: image.height / 2)
}

private func level(_ image: CGImage, x: Int, y: Int) throws -> Double {
  let pixel = try rgb(image, x: x, y: y)
  return (pixel.red + pixel.green + pixel.blue) / 3
}

private func rgb(_ image: CGImage, x: Int, y: Int) throws -> (red: Double, green: Double, blue: Double) {
  var data = [UInt8](repeating: 0, count: image.width * image.height * 4)
  let context = try #require(CGContext(
    data: &data, width: image.width, height: image.height, bitsPerComponent: 8, bytesPerRow: image.width * 4,
    space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
  ))
  context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
  let at = (y * image.width + x) * 4
  return (Double(data[at]) / 255, Double(data[at + 1]) / 255, Double(data[at + 2]) / 255)
}

private struct PictureFacts {
  var width: Int
  var height: Int
  var type: String?
  var tagged: Bool
  var image: CGImage
}

private func pictureFacts(_ url: URL) throws -> PictureFacts {
  let source = try #require(CGImageSourceCreateWithURL(url as CFURL, nil))
  let properties = try #require(CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any])
  let tiff = properties[kCGImagePropertyTIFFDictionary] as? [CFString: Any]
  let exif = properties[kCGImagePropertyExifDictionary] as? [CFString: Any]
  let tagged = properties[kCGImagePropertyGPSDictionary] != nil || tiff?[kCGImagePropertyTIFFMake] != nil || tiff?[kCGImagePropertyTIFFModel] != nil
    || exif?[kCGImagePropertyExifLensModel] != nil || exif?[kCGImagePropertyExifBodySerialNumber] != nil
  return PictureFacts(
    width: properties[kCGImagePropertyPixelWidth] as? Int ?? 0, height: properties[kCGImagePropertyPixelHeight] as? Int ?? 0,
    type: CGImageSourceGetType(source) as String?, tagged: tagged, image: try #require(CGImageSourceCreateImageAtIndex(source, 0, nil))
  )
}

private func makeJob(_ attempt: UInt64, _ source: URL, _ destination: URL, _ descriptor: Data) -> MediaPrepareJob {
  MediaPrepareJob(attemptId: attempt, source: source.path, destination: destination.path, descriptor: descriptor)
}

private func recodeLimits() -> VideoLimits {
  var limits = defaultVideoLimits()
  limits.maxLongSide = 320
  return limits
}

private func sizeOnDisk(_ url: URL) throws -> UInt64 {
  try #require(try FileManager.default.attributesOfItem(atPath: url.path)[.size] as? UInt64)
}

private func editDescriptor(_ edit: PhotoEdit, format: ImageFormat = .jpeg) throws -> Data {
  try mediaPrepareDescriptor(intent: .image(edit: edit, quality: 0.8, format: format))
}

// MARK: - The source matrix of the DBlayer executor, against the public MediaPreparer

@Suite(.serialized)
struct MediaPreparerMatrixTests {
  // DBlayer "h264+aac within limits" (private metadata stripped by a repack)
  @Test(.timeLimit(.minutes(2)))
  func aClipWithinLimitsAndPrivateMetadataIsRepackedWithoutIt() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "source.mp4")
    try await writeClip(FixtureClip(width: 64, height: 32, audio: true, metadata: true), to: source)
    let before = try await videoFacts(source)
    // As DBlayer's row: the source carries private metadata (an MP4 written by AVAssetWriter keeps the title, not always the location).
    #expect(!videoMetadataToStrip(keys: before.metadata).isEmpty, "the fixture carries private metadata: \(before.metadata)")
    let destination = directory.appending(path: "result.mp4")
    let outcome = await MediaPreparer().run(job: makeJob(10, source, destination, try videoDescriptor())) { _ in }
    guard case let .done(_, mime) = outcome else {
      Issue.record("the metadata forces a repack, got \(outcome)")
      return
    }
    #expect(mime == "video/mp4")
    let facts = try await videoFacts(destination)
    #expect(facts.width == 64 && facts.height == 32 && facts.codec == kCMVideoCodecType_H264 && facts.frames == 30)
    #expect(facts.audio && facts.audioCodec == kAudioFormatMPEG4AAC, "the AAC track stays")
    #expect(facts.metadata.allSatisfy { !$0.lowercased().contains("location") && !$0.lowercased().contains("songname") }, "private metadata is gone: \(facts.metadata)")
  }

  // DBlayer "h264 above limits"
  @Test(.timeLimit(.minutes(2)))
  func aClipAboveTheSideLimitIsRecodedSmallerAndKeepsItsAudio() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "source.mp4")
    try await writeClip(FixtureClip(width: 640, height: 360, audio: true), to: source)
    let destination = directory.appending(path: "result.mp4")
    let outcome = await MediaPreparer().run(job: makeJob(11, source, destination, try videoDescriptor(limits: recodeLimits()))) { _ in }
    guard case .done = outcome else {
      Issue.record("the clip is above the limit, got \(outcome)")
      return
    }
    let facts = try await videoFacts(destination)
    #expect(facts.width == 320 && facts.height == 180 && facts.codec == kCMVideoCodecType_H264 && facts.frames == 30)
    #expect(facts.audio && facts.audioCodec == kAudioFormatMPEG4AAC)
  }

  // A mirrored front take is re-encoded with the mirror in the pixels and an identity matrix (owner 2026-11-20: only a mirrored take is re-encoded).
  @Test(.timeLimit(.minutes(2)))
  func inv_capture_91_aMirroredTakeIsWrittenWithIdentityMatrixAndMirroredPixels() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "source.mp4")
    try await writeClip(FixtureClip(width: 128, height: 64, audio: true), to: source)
    let plan = try #require(captureUprightPlan(width: 128, height: 64, rotationDegrees: 0, mirror: true, bitrate: 500_000, frameRate: 30))
    let output = directory.appending(path: "upright.mp4")
    try await PrepareVideoWrite.run(
      source: source, output: output, plan: plan, audio: PrepareAudio(bitrate: 64_000, channels: 1, sampleRate: 44_100), mirror: true, entry: PrepareRunning(), report: { _ in }
    )
    let track = try #require(try await AVURLAsset(url: output).loadTracks(withMediaType: .video).first)
    let (size, transform) = try await track.load(.naturalSize, .preferredTransform)
    #expect(transform == .identity && Int(size.width) == 128 && Int(size.height) == 64)
    // The fixture is white on the left and black on the right: mirrored pixels hold the black half on the left.
    let (left, right) = (try await luminance(output, left: true), try await luminance(output, left: false))
    #expect(left < 0.3 && right > 0.7)
  }

  @Test
  func aTurnedTakeWithoutMirrorKeepsItsRotationTag() {
    // A plain sensor turn stays as the track matrix: every player and the server read it (owner 2026-11-20).
    #expect(captureUprightPlan(width: 128, height: 64, rotationDegrees: 90, mirror: false, bitrate: 500_000, frameRate: 30) == nil)
  }


  // DBlayer "hevc"
  @Test(.timeLimit(.minutes(2)))
  func anHevcClipIsRecodedToH264() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "source.mp4")
    do { try await writeClip(FixtureClip(width: 128, height: 64, codec: .hevc, audio: true), to: source) } catch { return }
    let destination = directory.appending(path: "result.mp4")
    let outcome = await MediaPreparer().run(job: makeJob(12, source, destination, try videoDescriptor())) { _ in }
    guard case .done = outcome else {
      Issue.record("an HEVC source is recoded, got \(outcome)")
      return
    }
    let facts = try await videoFacts(destination)
    #expect(facts.width == 128 && facts.height == 64 && facts.codec == kCMVideoCodecType_H264 && facts.frames == 30 && facts.audio)
  }

  // DBlayer "vertical turned 90" (rotation)
  @Test(.timeLimit(.minutes(2)))
  func aClipTurnedBy90DegreesIsWrittenUprightWithoutATurnFlag() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "source.mp4")
    try await writeClip(FixtureClip(width: 64, height: 32, audio: true, turned: true), to: source)
    let destination = directory.appending(path: "result.mp4")
    let outcome = await MediaPreparer().run(job: makeJob(13, source, destination, try videoDescriptor())) { _ in }
    guard case .done = outcome else {
      Issue.record("a turned clip is recoded, got \(outcome)")
      return
    }
    let facts = try await videoFacts(destination)
    #expect(facts.width == 32 && facts.height == 64 && facts.codec == kCMVideoCodecType_H264 && facts.frames == 30)
    let track = try #require(try await AVURLAsset(url: destination).loadTracks(withMediaType: .video).first)
    #expect(try await track.load(.preferredTransform).isIdentity, "the result carries no rotation")
  }

  // DBlayer "no audio track"
  @Test(.timeLimit(.minutes(2)))
  func aClipWithoutSoundIsRecodedWithoutAnAudioTrack() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "source.mp4")
    try await writeClip(FixtureClip(width: 640, height: 360), to: source)
    let destination = directory.appending(path: "result.mp4")
    let outcome = await MediaPreparer().run(job: makeJob(14, source, destination, try videoDescriptor(limits: recodeLimits()))) { _ in }
    guard case .done = outcome else {
      Issue.record("the silent clip is recoded, got \(outcome)")
      return
    }
    let facts = try await videoFacts(destination)
    #expect(facts.width == 320 && facts.height == 180 && facts.frames == 30 && !facts.audio)
  }

  // DBlayer "mirror"
  @Test(.timeLimit(.minutes(2)))
  func theMirrorFlagFlipsTheClipLeftToRight() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "source.mp4")
    try await writeClip(FixtureClip(width: 64, height: 32), to: source)
    #expect(try await luminance(source, left: true) > 0.7, "the source is bright on the left")
    let destination = directory.appending(path: "result.mp4")
    let outcome = await MediaPreparer().run(job: makeJob(15, source, destination, try videoDescriptor(mirror: true))) { _ in }
    guard case .done = outcome else {
      Issue.record("the mirror forces a recode, got \(outcome)")
      return
    }
    let left = try await luminance(destination, left: true)
    let right = try await luminance(destination, left: false)
    #expect(left < 0.3 && right > 0.7, "the left side is dark and the right side bright after the mirror: \(left) \(right)")
  }

  // DBlayer "hdr hlg hevc" (the fixture or the recode may be missing on the runtime)
  @Test(.timeLimit(.minutes(2)))
  func anHdrClipIsWrittenAsSdrBt709() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "source.mp4")
    do { try await writeClip(FixtureClip(width: 64, height: 32, codec: .hevc, hdr: true), to: source) } catch { return }
    let destination = directory.appending(path: "result.mp4")
    let outcome = await MediaPreparer().run(job: makeJob(16, source, destination, try videoDescriptor())) { _ in }
    switch outcome {
    case .done:
      #expect(try await videoFacts(destination).primaries == "ITU_R_709_2", "the result is SDR BT.709")
    case let .failed(message), let .cannotDecode(message):
      #expect(!message.isEmpty, "a runtime without HDR names its reason")
      #expect(!FileManager.default.fileExists(atPath: destination.path))
    default:
      Issue.record("an HDR clip is recoded or refused, got \(outcome)")
    }
  }

  // The recode threshold of the limits
  @Test(.timeLimit(.minutes(2)))
  func aSourceAtOrBelowTheRecodeThresholdPassesWhateverItsSize() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "source.mp4")
    try await writeClip(FixtureClip(width: 640, height: 360, audio: true), to: source)
    var limits = recodeLimits()
    limits.recodeAboveBytes = try sizeOnDisk(source)
    let destination = directory.appending(path: "result.mp4")
    let outcome = await MediaPreparer().run(job: makeJob(17, source, destination, try videoDescriptor(limits: limits))) { _ in }
    guard case .unchanged = outcome else {
      Issue.record("the source is at the threshold, got \(outcome)")
      return
    }
    #expect(try FileManager.default.contentsOfDirectory(atPath: directory.path).sorted() == ["source.mp4"])
  }

  @Test(.timeLimit(.minutes(2)))
  func aSourceAboveTheRecodeThresholdIsRecoded() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "source.mp4")
    try await writeClip(FixtureClip(width: 640, height: 360, audio: true), to: source)
    var limits = recodeLimits()
    limits.recodeAboveBytes = try sizeOnDisk(source) - 1
    let destination = directory.appending(path: "result.mp4")
    let outcome = await MediaPreparer().run(job: makeJob(18, source, destination, try videoDescriptor(limits: limits))) { _ in }
    guard case .done = outcome else {
      Issue.record("the source is above the threshold, got \(outcome)")
      return
    }
    #expect(try await videoFacts(destination).width == 320)
  }

  // The three results: pass (Unchanged), repack, recode
  @Test(.timeLimit(.minutes(2)))
  func theProbeAnnouncesTheSizeOfEachOfPassRepackAndRecode() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let plain = directory.appending(path: "plain.mp4")
    let tagged = directory.appending(path: "tagged.mp4")
    let large = directory.appending(path: "large.mp4")
    try await writeClip(FixtureClip(width: 64, height: 32), to: plain)
    try await writeClip(FixtureClip(width: 64, height: 32, audio: true, metadata: true), to: tagged)
    try await writeClip(FixtureClip(width: 640, height: 360), to: large)
    let preparer = MediaPreparer()
    let descriptor = try videoDescriptor(limits: recodeLimits())
    guard case let .file(plainWidth, plainHeight, plainMime) = await preparer.probe(path: plain.path, descriptor: descriptor, mime: nil),
          case let .file(taggedWidth, taggedHeight, _) = await preparer.probe(path: tagged.path, descriptor: descriptor, mime: nil),
          case let .file(largeWidth, largeHeight, _) = await preparer.probe(path: large.path, descriptor: descriptor, mime: nil)
    else {
      Issue.record("the probe reads the clips")
      return
    }
    #expect(plainWidth == 64 && plainHeight == 32 && plainMime == "video/mp4")
    #expect(taggedWidth == 64 && taggedHeight == 32, "a repack keeps the size")
    #expect(largeWidth == 320 && largeHeight == 180, "a recode gets the planned size")
  }

  // DBlayer probe turn: a turned source is reported upright
  @Test(.timeLimit(.minutes(1)))
  func theProbeReportsATurnedClipUpright() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "turned.mp4")
    try await writeClip(FixtureClip(width: 64, height: 32, audio: true, turned: true), to: source)
    guard case let .file(width, height, mime) = await MediaPreparer().probe(path: source.path, descriptor: nil, mime: nil) else {
      Issue.record("the probe reads the clip")
      return
    }
    #expect(width == 32 && height == 64 && mime == "video/mp4")
  }

  // Decoder unavailable: the source cannot be read, the policy decides
  @Test(.timeLimit(.minutes(1)))
  func anUnreadableVideoIsRefusedAsCannotDecodeByThePolicyFail() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "broken.mp4")
    try Data(repeating: 7, count: 2048).write(to: source)
    let destination = directory.appending(path: "result.mp4")
    let outcome = await MediaPreparer().run(job: makeJob(20, source, destination, try videoDescriptor())) { _ in }
    guard case let .cannotDecode(message) = outcome else {
      Issue.record("the source cannot be decoded, got \(outcome)")
      return
    }
    #expect(!message.isEmpty)
    #expect(!FileManager.default.fileExists(atPath: destination.path))
  }

  @Test(.timeLimit(.minutes(1)))
  func anUnreadableVideoPassesUnchangedByThePolicyUploadSource() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "broken.mp4")
    try Data(repeating: 7, count: 2048).write(to: source)
    let destination = directory.appending(path: "result.mp4")
    let descriptor = try mediaPrepareDescriptor(intent: .video(limits: nil, mirror: false, onCannotDecode: .uploadSource))
    let outcome = await MediaPreparer().run(job: makeJob(21, source, destination, descriptor)) { _ in }
    guard case .unchanged = outcome else {
      Issue.record("the original goes as it is, got \(outcome)")
      return
    }
    #expect(!FileManager.default.fileExists(atPath: destination.path))
    #expect(try FileManager.default.contentsOfDirectory(atPath: directory.path).sorted() == ["broken.mp4"])
  }

  @Test(.timeLimit(.minutes(1)))
  func aPhotoDescriptorOnAVideoSourceIsCannotDecode() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "clip.mp4")
    try await writeClip(source, width: 64, height: 32, frames: 3)
    let destination = directory.appending(path: "result.jpg")
    let outcome = await MediaPreparer().run(job: makeJob(22, source, destination, try editDescriptor(plainEdit))) { _ in }
    guard case .cannotDecode = outcome else {
      Issue.record("the kind does not match the descriptor, got \(outcome)")
      return
    }
    #expect(!FileManager.default.fileExists(atPath: destination.path))
  }

  // Cancel mid run with a real recode
  @Test(.timeLimit(.minutes(2)))
  func cancelDuringARecodeStopsItAndRemovesThePartialFile() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "long.mp4")
    try await writeClip(FixtureClip(width: 640, height: 360, audio: true, frames: 240), to: source)
    let destination = directory.appending(path: "result.mp4")
    let preparer = MediaPreparer()
    let first = AsyncStream<Double>.makeStream()
    let job = makeJob(23, source, destination, try videoDescriptor(limits: recodeLimits()))
    let work = Task {
      let outcome = await preparer.run(job: job) { first.continuation.yield($0) }
      first.continuation.finish()
      return outcome
    }
    var values = first.stream.makeAsyncIterator()
    if await values.next() == nil {
      Issue.record("the run ended before any progress: \(await work.value)")
      return
    }
    preparer.cancel(attemptId: 23)
    #expect(await work.value == .cancelled)
    #expect(!FileManager.default.fileExists(atPath: destination.path))
    #expect(try FileManager.default.contentsOfDirectory(atPath: directory.path).sorted() == ["long.mp4"], "no temp file is left")
  }

  // DBlayer picture rows: GPS and device data strip, format, orientation
  @Test(.timeLimit(.minutes(2)))
  func aTaggedJpegIsWrittenWithoutGpsAndDeviceData() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "tagged.jpg")
    try writePicture(source, width: 40, height: 20, orientation: 1, tagged: true)
    #expect(try pictureFacts(source).tagged, "the fixture carries GPS and device data")
    let destination = directory.appending(path: "result.jpg")
    let outcome = await MediaPreparer().run(job: makeJob(30, source, destination, try editDescriptor(plainEdit))) { _ in }
    guard case let .done(_, mime) = outcome else {
      Issue.record("the photo is prepared, got \(outcome)")
      return
    }
    let facts = try pictureFacts(destination)
    #expect(mime == "image/jpeg" && facts.type == UTType.jpeg.identifier)
    #expect(facts.width == 40 && facts.height == 20)
    #expect(!facts.tagged, "no GPS or device data is kept")
  }

  @Test(.timeLimit(.minutes(2)))
  func aTaggedPngWithAlphaStaysPngWithoutGpsAndDeviceData() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "alpha.png")
    try writePicture(source, type: .png, width: 40, height: 20, orientation: 1, alpha: true, tagged: true)
    let destination = directory.appending(path: "result.png")
    let outcome = await MediaPreparer().run(job: makeJob(31, source, destination, try editDescriptor(plainEdit, format: .png))) { _ in }
    guard case let .done(_, mime) = outcome else {
      Issue.record("the png is prepared, got \(outcome)")
      return
    }
    let facts = try pictureFacts(destination)
    #expect(mime == "image/png" && facts.type == UTType.png.identifier)
    #expect(facts.width == 40 && facts.height == 20 && !facts.tagged)
  }

  // DBlayer "heic" (the encoder may be missing on the runtime)
  @Test(.timeLimit(.minutes(2)))
  func aTaggedHeicIsWrittenAsJpegWithoutGpsAndDeviceData() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "tagged.heic")
    do { try writePicture(source, type: UTType("public.heic") ?? .jpeg, width: 40, height: 20, orientation: 1, tagged: true) } catch { return }
    let destination = directory.appending(path: "result.jpg")
    let outcome = await MediaPreparer().run(job: makeJob(32, source, destination, try editDescriptor(plainEdit))) { _ in }
    switch outcome {
    case .done:
      let facts = try pictureFacts(destination)
      #expect(facts.type == UTType.jpeg.identifier && facts.width == 40 && facts.height == 20 && !facts.tagged)
    case let .failed(message), let .cannotDecode(message):
      #expect(!message.isEmpty)
      #expect(!FileManager.default.fileExists(atPath: destination.path))
    default:
      Issue.record("a HEIC photo is prepared or refused, got \(outcome)")
    }
  }

  // DBlayer "jpeg exif orientation 6" (rotation)
  @Test(.timeLimit(.minutes(2)))
  func anExifTurnedJpegIsWrittenUprightWithItsColorsOnTheRightSides() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "turned.jpg")
    try writePicture(source, width: 40, height: 20, orientation: 6, tagged: true)
    let destination = directory.appending(path: "result.jpg")
    let outcome = await MediaPreparer().run(job: makeJob(33, source, destination, try editDescriptor(plainEdit))) { _ in }
    guard case .done = outcome else {
      Issue.record("the photo is prepared, got \(outcome)")
      return
    }
    let facts = try pictureFacts(destination)
    #expect(facts.width == 20 && facts.height == 40 && !facts.tagged)
    // Orientation 6 turns the picture 90 degrees clockwise: the red left half ends up on top.
    let top = try rgb(facts.image, x: 10, y: 5)
    let bottom = try rgb(facts.image, x: 10, y: 35)
    #expect(top.red > 0.6 && top.blue < 0.4, "the red half is on top: \(top)")
    #expect(bottom.blue > 0.6 && bottom.red < 0.4, "the blue half is below: \(bottom)")
  }

  // Crop and the descriptor rotation
  @Test(.timeLimit(.minutes(2)))
  func aCropKeepsOnlyTheChosenPartOfThePhoto() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "photo.jpg")
    try writePicture(source, width: 40, height: 20, orientation: 1, tagged: true)
    let edit = PhotoEdit(maxSize: nil, enlarge: false, crop: CropFractions(x: 0.5, y: 0, width: 0.5, height: 1), outputSize: nil, mirror: false, rotationDegrees: 0)
    let descriptor = try editDescriptor(edit)
    guard case let .file(plannedWidth, plannedHeight, _) = await MediaPreparer().probe(path: source.path, descriptor: descriptor, mime: nil) else {
      Issue.record("the probe plans the crop")
      return
    }
    let destination = directory.appending(path: "result.jpg")
    let outcome = await MediaPreparer().run(job: makeJob(34, source, destination, descriptor)) { _ in }
    guard case .done = outcome else {
      Issue.record("the cropped photo is prepared, got \(outcome)")
      return
    }
    let facts = try pictureFacts(destination)
    #expect(facts.width == 20 && facts.height == 20)
    #expect(UInt32(facts.width) == plannedWidth && UInt32(facts.height) == plannedHeight, "the file has the size the probe announced")
    let pixel = try rgb(facts.image, x: 10, y: 10)
    #expect(pixel.blue > 0.6 && pixel.red < 0.4, "only the blue right half is left: \(pixel)")
    #expect(!facts.tagged)
  }

  @Test(.timeLimit(.minutes(2)))
  func aRotationOfTheEditTurnsThePhotoBy90Degrees() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "photo.jpg")
    try writePicture(source, width: 40, height: 20, orientation: 1)
    let edit = PhotoEdit(maxSize: nil, enlarge: false, crop: nil, outputSize: nil, mirror: false, rotationDegrees: 90)
    let destination = directory.appending(path: "result.jpg")
    let outcome = await MediaPreparer().run(job: makeJob(35, source, destination, try editDescriptor(edit))) { _ in }
    guard case .done = outcome else {
      Issue.record("the turned photo is prepared, got \(outcome)")
      return
    }
    let facts = try pictureFacts(destination)
    #expect(facts.width == 20 && facts.height == 40)
  }

  @Test(.timeLimit(.minutes(1)))
  func aPhotoThatCannotBeDecodedIsCannotDecodeAndWritesNothing() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "broken.jpg")
    try Data(repeating: 7, count: 2048).write(to: source)
    let destination = directory.appending(path: "result.jpg")
    let outcome = await MediaPreparer().run(job: makeJob(36, source, destination, try editDescriptor(plainEdit))) { _ in }
    guard case .cannotDecode = outcome else {
      Issue.record("the source is no picture, got \(outcome)")
      return
    }
    #expect(!FileManager.default.fileExists(atPath: destination.path))
  }
}

// MARK: - A prepared JPEG flattens transparency onto white, like Android and encodeImage

private func writeHalfTransparentPng(_ url: URL, width: Int, height: Int) throws {
  let context = try #require(CGContext(
    data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(),
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
  ))
  context.clear(CGRect(x: 0, y: 0, width: width, height: height))
  context.setFillColor(CGColor(red: 1, green: 0, blue: 0, alpha: 1))
  context.fill(CGRect(x: width / 2, y: 0, width: width - width / 2, height: height))
  let destination = try #require(CGImageDestinationCreateWithURL(url as CFURL, UTType.png.identifier as CFString, 1, nil))
  CGImageDestinationAddImage(destination, try #require(context.makeImage()), nil)
  try #require(CGImageDestinationFinalize(destination))
}

@Suite(.serialized)
struct MediaPrepareTransparencyTests {
  @Test(.timeLimit(.minutes(2)))
  func inv_send_65_aPreparedJpegFlattensTransparencyOntoWhite() async throws {
    let directory = try scratch()
    defer { try? FileManager.default.removeItem(at: directory) }
    let source = directory.appending(path: "alpha.png")
    try writeHalfTransparentPng(source, width: 40, height: 20)
    let straight = directory.appending(path: "straight.jpg")
    let straightOutcome = await MediaPreparer().run(job: makeJob(70, source, straight, try editDescriptor(plainEdit))) { _ in }
    guard case .done = straightOutcome else {
      Issue.record("the image is prepared, got \(straightOutcome)")
      return
    }
    let straightImage = try pictureFacts(straight).image
    let clear = try rgb(straightImage, x: 5, y: 10)
    #expect(clear.red >= 0.98 && clear.green >= 0.98 && clear.blue >= 0.98, "transparent region is white, got \(clear)")
    // Red goes through the colour conversion of the JPEG encoder: about 0.15 green comes back, still clearly red.
    let solid = try rgb(straightImage, x: 35, y: 10)
    #expect(solid.red >= 0.9 && solid.green <= 0.3 && solid.blue <= 0.3, "the opaque half stays red, got \(solid)")

    let turned = directory.appending(path: "turned.jpg")
    let turnedEdit = PhotoEdit(maxSize: nil, enlarge: false, crop: nil, outputSize: nil, mirror: false, rotationDegrees: 90)
    let turnedOutcome = await MediaPreparer().run(job: makeJob(71, source, turned, try editDescriptor(turnedEdit))) { _ in }
    guard case .done = turnedOutcome else {
      Issue.record("the turned image is prepared, got \(turnedOutcome)")
      return
    }
    let turnedImage = try pictureFacts(turned).image
    #expect(turnedImage.width == 20 && turnedImage.height == 40)
    // turned 90 degrees clockwise the left (transparent) half is on top, the red half at the bottom
    let top = try rgb(turnedImage, x: 10, y: 5)
    #expect(top.red >= 0.98 && top.green >= 0.98 && top.blue >= 0.98, "turned transparent region is white, got \(top)")
    let bottom = try rgb(turnedImage, x: 10, y: 35)
    #expect(bottom.red >= 0.9 && bottom.green <= 0.3 && bottom.blue <= 0.3, "the turned opaque half stays red, got \(bottom)")
  }
}
