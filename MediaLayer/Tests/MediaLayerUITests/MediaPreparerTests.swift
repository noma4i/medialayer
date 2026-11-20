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
    writer.metadata = [(AVMetadataIdentifier.quickTimeMetadataLocationISO6709, "+37.3300-122.0300/"), (.iTunesMetadataSongName, "Private title")].map {
      let item = AVMutableMetadataItem()
      item.identifier = $0.0
      item.value = $0.1 as NSString
      item.dataType = "com.apple.metadata.datatype.UTF-8"
      return item
    }
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
  await feed(video, count: clip.frames) { index in
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
  await writer.finishWriting()
  try #require(writer.status == .completed, "the fixture clip cannot be written: \(String(describing: writer.error))")
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
