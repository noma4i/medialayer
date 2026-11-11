import AVFoundation
import CryptoKit
import CoreGraphics
import Foundation
import ImageIO
@testable import MediaLayer
import MediaLayerTesting
import Testing
@testable import MediaLayerUI

private func fixture(_ name: String, _ fileExtension: String, kind: Kind = .video) -> MediaSourceValue {
  .file(Bundle.module.url(forResource: name, withExtension: fileExtension, subdirectory: "Fixtures")!, kind: kind)
}

private func open() async throws -> MediaLayer {
  try await MediaLayer.open(
    fetcher: TestFetcher(),
    settings: UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!),
    directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
  )
}

private func smallLimits() -> VideoLimits {
  var limits = defaultVideoLimits()
  limits.maxLongSide = 48
  return limits
}

private func preparedFiles() -> Set<String> {
  let names = (try? FileManager.default.contentsOfDirectory(atPath: FileManager.default.temporaryDirectory.path)) ?? []
  return Set(names.filter { $0.hasSuffix(".mp4") || $0.hasSuffix(".jpg") })
}

private func clipWithLocation() async throws -> MediaSourceValue {
  let url = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).mp4")
  let writer = try AVAssetWriter(outputURL: url, fileType: .mp4)
  writer.metadata = [(AVMetadataIdentifier.quickTimeMetadataLocationISO6709, "+37.3300-122.0300/"), (.iTunesMetadataSongName, "Private title")].map {
    let item = AVMutableMetadataItem()
    item.identifier = $0.0
    item.value = $0.1 as NSString
    item.dataType = "com.apple.metadata.datatype.UTF-8"
    return item
  }
  let input = AVAssetWriterInput(mediaType: .video, outputSettings: [AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: 64, AVVideoHeightKey: 32])
  let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: nil)
  writer.add(input)
  writer.startWriting()
  writer.startSession(atSourceTime: .zero)
  for frame in 0..<30 {
    while !input.isReadyForMoreMediaData { try await Task.sleep(for: .milliseconds(5)) }
    var buffer: CVPixelBuffer?
    CVPixelBufferCreate(nil, 64, 32, kCVPixelFormatType_32BGRA, nil, &buffer)
    adaptor.append(try #require(buffer), withPresentationTime: CMTime(value: CMTimeValue(frame), timescale: 30))
  }
  input.markAsFinished()
  await writer.finishWriting()
  try #require(writer.status == .completed)
  return .file(url, kind: .video)
}

private func metadataIdentifiers(_ url: URL) async throws -> [String] {
  let asset = AVURLAsset(url: url)
  var items: [AVMetadataItem] = []
  for format in try await asset.load(.availableMetadataFormats) { items += try await asset.loadMetadata(for: format) }
  for track in try await asset.load(.tracks) { items += try await track.load(.metadata) }
  return items.compactMap { $0.identifier.map { decodedMetadataKey($0.rawValue) } }
}

private final class Marks: @unchecked Sendable {
  private let lock = NSLock()
  private var values: [Double] = []
  private var task: Task<PreparedMedia, Error>?
  var all: [Double] { lock.withLock { values } }
  func add(_ value: Double) { lock.withLock { values.append(value) } }
  func hold(_ task: Task<PreparedMedia, Error>) { lock.withLock { self.task = task } }
  func cancelOnFirstSample(_ value: Double) {
    add(value)
    for _ in 0 ..< 200 {
      if let task = lock.withLock({ task }) {
        task.cancel()
        return
      }
      Thread.sleep(forTimeInterval: 0.01)
    }
  }
}

private final class TeardownProbe: @unchecked Sendable {
  private let lock = NSLock()
  private var task: Task<PreparedMedia, Error>?
  private var inside = false
  private var cancelled = false
  private var count = 0
  private var insideAtTeardown = false
  var tearDownCount: Int { lock.withLock { count } }
  var tearDownRanInsideCallback: Bool { lock.withLock { insideAtTeardown } }
  func hold(_ task: Task<PreparedMedia, Error>) { lock.withLock { self.task = task } }
  func cancelInsideCallback() {
    let target: Task<PreparedMedia, Error>? = lock.withLock {
      guard !cancelled, let task else { return nil }
      cancelled = true
      inside = true
      return task
    }
    target?.cancel()
    lock.withLock { inside = false }
  }
  func tornDown() { lock.withLock { count += 1; insideAtTeardown = insideAtTeardown || inside } }
}

private func pixel(_ url: URL, x: Int, y: Int) throws -> (r: Int, g: Int, b: Int, width: Int, height: Int) {
  let source = try #require(CGImageSourceCreateWithURL(url as CFURL, nil))
  let image = try #require(CGImageSourceCreateImageAtIndex(source, 0, nil))
  var data = [UInt8](repeating: 0, count: image.width * image.height * 4)
  let context = try #require(CGContext(
    data: &data, width: image.width, height: image.height, bitsPerComponent: 8, bytesPerRow: image.width * 4,
    space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
  ))
  context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
  try #require(x < image.width && y < image.height, "the point \(x),\(y) is outside the \(image.width)x\(image.height) image")
  let at = (y * image.width + x) * 4
  return (Int(data[at]), Int(data[at + 1]), Int(data[at + 2]), image.width, image.height)
}

extension GlobalMedia {
  @Suite(.serialized)
  struct PrepareMediaTests {
    @Test
    func inv_import_01_imported_image_and_video_are_accepted_by_preparation() async throws {
      let media = try await open()
      for (name, ext, kind) in [("exif6_editor", "jpg", Kind.image), ("silent", "mp4", Kind.video)] {
        let source = Bundle.module.url(forResource: name, withExtension: ext, subdirectory: "Fixtures")!
        let imported = try await media.importMedia(source)
        defer { try? FileManager.default.removeItem(at: imported.url) }
        #expect(imported.isNew)
        #expect(imported.url != source)
        let value = MediaSourceValue.file(imported.url, kind: kind)
        let prepared = try await (kind == .image ? media.prepareImage(value) : media.prepareVideo(value))
        if prepared.isNew { try? FileManager.default.removeItem(at: prepared.url) }
        #expect(FileManager.default.fileExists(atPath: imported.url.path))
      }
    }

    @Test
    func inv_import_02_large_file_preserves_the_sha256() async throws {
      let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
      try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
      defer { try? FileManager.default.removeItem(at: directory) }
      let source = directory.appendingPathComponent("large.mov")
      FileManager.default.createFile(atPath: source.path, contents: nil)
      let writer = try FileHandle(forWritingTo: source)
      let chunk = Data(repeating: 0x97, count: 64 * 1024)
      var hash = SHA256()
      for _ in 0..<256 { try writer.write(contentsOf: chunk); hash.update(data: chunk) }
      try writer.close()
      var requests: [Int] = []
      let imported = try importMediaFile(source, directory: directory, read: { handle, count in
        requests.append(count)
        return try handle.read(upToCount: count)
      })
      let reader = try FileHandle(forReadingFrom: imported.url)
      defer { try? reader.close() }
      var copied = SHA256()
      while let bytes = try reader.read(upToCount: 64 * 1024), !bytes.isEmpty { copied.update(data: bytes) }
      #expect(hash.finalize() == copied.finalize())
      #expect(imported.url.pathExtension == "mov")
      #expect(requests.count > 256)
      #expect(requests.allSatisfy { $0 <= 64 * 1024 })
      #expect(try FileManager.default.attributesOfItem(atPath: imported.url.path)[.size] as? Int == 16 * 1024 * 1024)
    }

    @Test
    func inv_import_03_open_read_and_write_errors_are_explicit() throws {
      let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
      #expect(throws: MediaError.self) { try importMediaFile(directory) }
      let source = Bundle.module.url(forResource: "silent", withExtension: "mp4", subdirectory: "Fixtures")!
      #expect(throws: MediaError.self) { try importMediaFile(source, directory: directory) }
      #expect(!FileManager.default.fileExists(atPath: directory.path))
      try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
      defer { try? FileManager.default.removeItem(at: directory) }
      var input: FileHandle?
      var output: FileHandle?
      #expect(throws: MediaError.self) {
        try importMediaFile(source, directory: directory, read: { handle, _ in
          input = handle
          throw CocoaError(.fileReadUnknown)
        })
      }
      #expect(throws: (any Error).self) { try input!.offset() }
      #expect(try FileManager.default.contentsOfDirectory(atPath: directory.path).isEmpty)
      #expect(throws: MediaError.self) {
        try importMediaFile(source, directory: directory, write: { handle, bytes in
          output = handle
          try handle.write(contentsOf: bytes.prefix(1))
          throw CocoaError(.fileWriteOutOfSpace)
        })
      }
      #expect(throws: (any Error).self) { try output!.offset() }
      #expect(try FileManager.default.contentsOfDirectory(atPath: directory.path).isEmpty)
    }

    @Test
    func inv_import_04_cancellation_closes_streams_and_removes_only_the_partial_file() async throws {
      let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
      try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
      defer { try? FileManager.default.removeItem(at: directory) }
      let source = directory.appendingPathComponent("source.mp4")
      try Data(repeating: 0x97, count: 128 * 1024).write(to: source)
      let finished = try importMediaFile(source, directory: directory)
      let attempt = Task.detached { () throws -> Bool in
        var input: FileHandle?
        var output: FileHandle?
        var reads = 0
        do {
          _ = try importMediaFile(source, directory: directory, read: { handle, count in
            input = handle
            reads += 1
            return try handle.read(upToCount: count)
          }, write: { handle, bytes in
            output = handle
            try handle.write(contentsOf: bytes)
            withUnsafeCurrentTask { $0?.cancel() }
          })
          Issue.record("a cancelled copy returned a file")
        } catch is CancellationError {
          #expect(reads == 1)
          #expect(throws: (any Error).self) { try input!.offset() }
          #expect(throws: (any Error).self) { try output!.offset() }
          return true
        }
        return false
      }
      #expect(try await attempt.value)
      #expect(Set(try FileManager.default.contentsOfDirectory(atPath: directory.path)) == Set([source.lastPathComponent, finished.url.lastPathComponent]))
      #expect(try Data(contentsOf: finished.url) == Data(contentsOf: source))
    }
    @Test(.timeLimit(.minutes(2)))
    func inv_send_10_a_video_above_the_table_is_encoded_upright_with_even_sides_and_its_length() async throws {
      let media = try await open()
      let marks = Marks()

      let prepared = try await media.prepareVideo(fixture("rotated90", "mp4"), limits: smallLimits(), progress: { marks.add($0) })
      defer { try? FileManager.default.removeItem(at: prepared.url) }

      let info = try #require(await media.mediaInfo(MediaSourceValue.file(prepared.url, kind: .video)))
      #expect(prepared.isNew)
      #expect(prepared.url.pathExtension == "mp4")
      #expect(info.frame == ImageSize(width: 24, height: 48))
      #expect(abs(Int(try #require(info.durationMs)) - 1500) <= 300)
      #expect(marks.all.last == 1)
      #expect(marks.all == marks.all.sorted())
    }

    @Test
    func inv_send_06_a_metadata_key_escaped_as_a_latin_byte_keeps_its_character_and_stays_technical() {
      #expect(decodedMetadataKey("itsk/%A9too") == "itsk/\u{A9}too")
      #expect(decodedMetadataKey("udta/%C2%A9xyz") == "udta/\u{A9}xyz")
      #expect(decodedMetadataKey("mdta/com.apple.quicktime.location.ISO6709") == "mdta/com.apple.quicktime.location.ISO6709")
      #expect(videoMetadataToStrip(keys: [decodedMetadataKey("itsk/%A9too")]).isEmpty)
      #expect(videoMetadataToStrip(keys: [decodedMetadataKey("udta/%A9xyz")]) == ["udta/\u{A9}xyz"])
    }

    @Test
    func inv_send_06_a_video_that_already_fits_comes_back_untouched() async throws {
      let media = try await open()
      let source = fixture("silent", "mp4")
      let before = preparedFiles()

      let prepared = try await media.prepareVideo(source)

      let metadata = try await metadataIdentifiers(URL(string: source.url)!)
      #expect(!prepared.isNew, "source metadata: \(metadata), private keys: \(videoMetadataToStrip(keys: metadata))")
      #expect(prepared.url.absoluteString == source.url)
      #expect(preparedFiles() == before)
    }

    @Test(.timeLimit(.minutes(2)))
    func inv_send_39_a_video_that_fits_but_carries_location_is_copied_without_encoding_and_without_the_tags() async throws {
      let media = try await open()
      let source = try await clipWithLocation()
      defer { try? FileManager.default.removeItem(at: URL(string: source.url)!) }
      let before = try await metadataIdentifiers(URL(string: source.url)!)
      try #require(before.contains { $0.hasSuffix("loci") }, "the fixture must carry a location")

      let prepared = try await media.prepareVideo(source)

      #expect(prepared.isNew)
      #expect(prepared.url.absoluteString != source.url)
      #expect(try await metadataIdentifiers(prepared.url).isEmpty)
      #expect(try await AVURLAsset(url: prepared.url).load(.duration).seconds == 1)
      try? FileManager.default.removeItem(at: prepared.url)
    }

    @Test(.timeLimit(.minutes(2)))
    func inv_send_40_an_encoded_video_does_not_carry_the_location_of_its_source() async throws {
      let media = try await open()
      let source = try await clipWithLocation()
      defer { try? FileManager.default.removeItem(at: URL(string: source.url)!) }
      try #require(!(try await metadataIdentifiers(URL(string: source.url)!)).isEmpty)

      let prepared = try await media.prepareVideo(source, limits: smallLimits())

      #expect(prepared.isNew)
      #expect(try await metadataIdentifiers(prepared.url).isEmpty)
      try? FileManager.default.removeItem(at: prepared.url)
    }

    @Test
    func inv_send_12_a_photo_with_an_exif_turn_becomes_an_upright_jpeg_without_the_editor_tag() async throws {
      let media = try await open()
      let source = fixture("exif6_editor", "jpg", kind: .image)

      let prepared = try await media.prepareImage(source)
      defer { try? FileManager.default.removeItem(at: prepared.url) }

      let bytes = try Data(contentsOf: prepared.url)
      let green = try pixel(prepared.url, x: 15, y: 5)
      let blue = try pixel(prepared.url, x: 5, y: 35)
      let image = try #require(CGImageSourceCreateWithURL(prepared.url as CFURL, nil))
      let properties = try #require(CGImageSourceCopyPropertiesAtIndex(image, 0, nil) as? [CFString: Any])
      #expect(prepared.isNew)
      #expect((green.width, green.height) == (20, 40))
      #expect(green.g > 150 && green.r < 80 && green.b < 80)
      #expect(blue.b > 150 && blue.r < 80 && blue.g < 80)
      #expect(((properties[kCGImagePropertyOrientation] as? Int) ?? 1) == 1)
      #expect(bytes.range(of: Data("CreatorTool".utf8)) == nil)
      #expect(bytes.prefix(3) == Data([0xFF, 0xD8, 0xFF]))
    }

    @Test
    func inv_send_12_the_quality_of_the_photo_comes_from_the_parameter() async throws {
      let media = try await open()
      let source = fixture("exif6_editor", "jpg", kind: .image)

      let low = try await media.prepareImage(source, quality: 0.05)
      let high = try await media.prepareImage(source, quality: 1)
      defer { try? FileManager.default.removeItem(at: low.url); try? FileManager.default.removeItem(at: high.url) }

      #expect(try Data(contentsOf: low.url).count < Data(contentsOf: high.url).count)
    }

    @Test(.timeLimit(.minutes(2)))
    func inv_send_13_cancelling_stops_the_encoding_and_leaves_no_file() async throws {
      let media = try await open()
      let marks = Marks()
      let before = preparedFiles()

      let task = Task { try await media.prepareVideo(fixture("rotated90", "mp4"), limits: smallLimits(), progress: { marks.cancelOnFirstSample($0) }) }
      marks.hold(task)

      await #expect(throws: CancellationError.self) { try await task.value }
      #expect(preparedFiles() == before)
    }

    @Test(.timeLimit(.minutes(2)))
    func inv_send_13_a_cancel_from_the_progress_callback_tears_down_after_the_callback_returns() async throws {
      let probe = TeardownProbe()
      let file = Bundle.module.url(forResource: "rotated90", withExtension: "mp4", subdirectory: "Fixtures")!
      let task = Task {
        try await VideoPreparation.run(
          file, limits: smallLimits(), progress: { _ in probe.cancelInsideCallback() }, tornDown: { probe.tornDown() }
        )
      }
      probe.hold(task)

      await #expect(throws: CancellationError.self) { try await task.value }
      #expect(probe.tearDownCount == 1)
      #expect(probe.tearDownRanInsideCallback == false, "the reader was torn down inside the sample callback")
    }

    @Test(.timeLimit(.minutes(1)))
    func inv_send_14_encoding_does_not_need_the_main_thread() async throws {
      let media = try await open()
      let gate = DispatchSemaphore(value: 0)
      DispatchQueue.main.async { _ = gate.wait(timeout: .now() + 8) }
      defer { gate.signal() }

      let started = Date()
      let video = try await media.prepareVideo(fixture("rotated90", "mp4"), limits: smallLimits())
      let photo = try await media.prepareImage(fixture("exif6_editor", "jpg", kind: .image))
      defer { try? FileManager.default.removeItem(at: video.url); try? FileManager.default.removeItem(at: photo.url) }

      #expect(Date().timeIntervalSince(started) < 6, "the work waited for the blocked main thread")
    }

    @Test
    func inv_send_15_a_source_that_is_not_a_local_file_is_refused() async throws {
      let media = try await open()
      let remote = MediaSourceValue.remote("https://o.test/a.mp4", kind: .video)

      await #expect(throws: MediaError.self) { try await media.prepareVideo(remote) }
      await #expect(throws: MediaError.self) { try await media.prepareImage(remote) }
    }
  }
}
