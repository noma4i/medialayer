@preconcurrency import AVFoundation
import CoreGraphics
import Foundation
import ImageIO
import MediaLayer
import Testing
import UniformTypeIdentifiers
@testable import MediaLayerUI

private func writeClip(_ url: URL, width: Int, height: Int, frames: Int) async throws {
  let writer = try AVAssetWriter(outputURL: url, fileType: .mp4)
  let video = AVAssetWriterInput(mediaType: .video, outputSettings: [AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: width, AVVideoHeightKey: height])
  writer.add(video)
  let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: video, sourcePixelBufferAttributes: nil)
  try #require(writer.startWriting())
  writer.startSession(atSourceTime: .zero)
  nonisolated(unsafe) let input = video
  nonisolated(unsafe) let feed = adaptor
  await withCheckedContinuation { (done: CheckedContinuation<Void, Never>) in
    nonisolated(unsafe) var next = 0
    input.requestMediaDataWhenReady(on: DispatchQueue(label: "preparer.fixture")) {
      while input.isReadyForMoreMediaData, next < frames {
        var buffer: CVPixelBuffer?
        CVPixelBufferCreate(nil, width, height, kCVPixelFormatType_32BGRA, nil, &buffer)
        if let buffer { feed.append(buffer, withPresentationTime: CMTime(value: CMTimeValue(next), timescale: 30)) }
        next += 1
      }
      if next >= frames {
        input.markAsFinished()
        done.resume()
      }
    }
  }
  await writer.finishWriting()
  try #require(writer.status == .completed)
}

private func writePicture(_ url: URL, width: Int, height: Int, orientation: UInt32) throws {
  let context = try #require(CGContext(
    data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0, space: CGColorSpaceCreateDeviceRGB(),
    bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
  ))
  context.setFillColor(CGColor(red: 1, green: 0, blue: 0, alpha: 1))
  context.fill(CGRect(x: 0, y: 0, width: width, height: height))
  let destination = try #require(CGImageDestinationCreateWithURL(url as CFURL, UTType.jpeg.identifier as CFString, 1, nil))
  CGImageDestinationAddImage(destination, try #require(context.makeImage()), [kCGImagePropertyOrientation: orientation] as CFDictionary)
  try #require(CGImageDestinationFinalize(destination))
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
