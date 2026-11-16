import CoreGraphics
import Foundation
import ImageIO
import MediaLayer
import MediaLayerTesting
import Testing
import UniformTypeIdentifiers
@testable import MediaLayerUI

private func jpeg(_ width: Int, _ height: Int, orientation: UInt32) throws -> Data {
  let context = try #require(CGContext(
    data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
  ))
  context.setFillColor(CGColor(red: 1, green: 0, blue: 0, alpha: 1))
  context.fill(CGRect(x: 0, y: 0, width: width, height: height))
  let data = NSMutableData()
  let destination = try #require(CGImageDestinationCreateWithData(data, UTType.jpeg.identifier as CFString, 1, nil))
  CGImageDestinationAddImage(destination, try #require(context.makeImage()), [kCGImagePropertyOrientation: orientation] as CFDictionary)
  #expect(CGImageDestinationFinalize(destination))
  return data as Data
}

private final class Admissions: @unchecked Sendable {
  private let lock = NSLock()
  private var items: [UInt64] = []
  private var mains: [Bool] = []
  private var releases = 0
  var asked: [UInt64] { lock.withLock { items } }
  var onMain: [Bool] { lock.withLock { mains } }
  var released: Int { lock.withLock { releases } }

  var admit: DecodeAdmit {
    { bytes in
      self.lock.withLock {
        self.items.append(bytes)
        self.mains.append(Thread.isMainThread)
      }
      return { self.lock.withLock { self.releases += 1 } }
    }
  }
}

extension GlobalMedia {
@Suite(.serialized, .timeLimit(.minutes(1)))
struct MediaImageDecodeTests {
  private func decode(_ data: Data, maxSide: Int?, circle: Bool = false, admit: @escaping DecodeAdmit) async throws -> PlatformImage {
    try await MediaImageDecode.decode(maxSide: maxSide, circle: circle, admit: admit) { CGImageSourceCreateWithData(data as CFData, nil) }
  }

  private func size(_ image: PlatformImage) throws -> [Int] {
    let cg = try #require(image.cgImage)
    return [cg.width, cg.height]
  }

  @Test
  func inv_image_70_an_exif_turn_of_six_and_eight_gives_an_upright_image_with_swapped_sides() async throws {
    let admissions = Admissions()
    for tag: UInt32 in [6, 8] {
      let image = try await decode(try jpeg(40, 20, orientation: tag), maxSide: nil, admit: admissions.admit)
      #expect(try size(image) == [20, 40], "tag \(tag)")
    }
    #expect(try size(try await decode(try jpeg(40, 20, orientation: 1), maxSide: nil, admit: admissions.admit)) == [40, 20])
  }

  @Test
  func inv_image_71_the_long_side_is_cut_to_the_limit_and_a_small_picture_is_kept() async throws {
    let admissions = Admissions()
    let cut = try await decode(try jpeg(400, 200, orientation: 6), maxSide: 100, admit: admissions.admit)
    let kept = try await decode(try jpeg(40, 20, orientation: 1), maxSide: 500, admit: admissions.admit)

    #expect(try size(cut) == [50, 100])
    #expect(try size(kept) == [40, 20])
  }

  @Test
  func inv_image_70_the_decode_runs_off_the_main_thread_and_asks_the_ledger_for_the_turned_raster() async throws {
    let admissions = Admissions()

    _ = try await decode(try jpeg(40, 20, orientation: 6), maxSide: nil, admit: admissions.admit)

    #expect(admissions.onMain == [false])
    #expect(admissions.asked == [40 * 20 * 4 * 2])
    #expect(admissions.released == 1)
  }

  @Test
  func inv_image_72_a_refused_admission_fails_the_decode_with_a_media_error_and_holds_nothing() async throws {
    let admissions = Admissions()
    let data = try jpeg(40, 20, orientation: 1)

    await #expect(throws: MediaError.self) {
      try await decode(data, maxSide: nil) { _ in throw MediaError.Overloaded(detail: "no room") }
    }
    await #expect(throws: MediaError.self) {
      try await decode(Data([1, 2, 3]), maxSide: nil, admit: admissions.admit)
    }

    #expect(admissions.asked.isEmpty)
  }

  @Test
  func inv_image_72_without_an_open_layer_the_public_decode_is_closed() async throws {
    Media.current = nil
    let data = try jpeg(40, 20, orientation: 1)

    await #expect(throws: MediaError.self) { try await MediaImageDecode.decode(data: data, maxSide: 10) }
  }

  @Test
  func inv_image_73_a_loaded_bitmap_is_cut_to_the_side_and_a_circle_is_a_square_with_transparent_corners() async throws {
    let admissions = Admissions()
    let data = try jpeg(80, 40, orientation: 1)

    let plain = try await MediaImageLoad.bitmap(of: data, side: 20, circle: false, admit: admissions.admit)
    let round = try await MediaImageLoad.bitmap(of: data, side: 20, circle: true, admit: admissions.admit)

    #expect(try size(plain) == [20, 10])
    #expect(try size(round) == [20, 20])
    let cg = try #require(round.cgImage)
    let pixels = try #require(cg.dataProvider?.data as Data?)
    let alpha = { (x: Int, y: Int) in pixels[y * cg.bytesPerRow + x * 4 + 3] }
    #expect(alpha(0, 0) == 0)
    #expect(alpha(10, 10) == 255)
    #expect(admissions.released == 2)
  }

  @Test
  func inv_image_73_load_bitmap_works_without_an_open_layer() async throws {
    Media.current = nil
    let address = "https://img.example/avatar.jpg"
    let fetcher = TestFetcher()
    fetcher.serve(try jpeg(80, 40, orientation: 6), at: address)
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)

    let image = try await MediaImageLoad.loadBitmap(url: URL(string: address)!, directory: directory, side: 30, fetcher: fetcher)

    #expect(try size(image) == [15, 30])
  }
}
}
