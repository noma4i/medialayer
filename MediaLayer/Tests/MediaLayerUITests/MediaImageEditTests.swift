import CoreGraphics
import Foundation
import ImageIO
import MediaLayer
import MediaLayerTesting
import Testing
import UIKit
import UniformTypeIdentifiers
@testable import MediaLayer
@testable import MediaLayerUI

private func halves(_ width: Int, _ height: Int, orientation: UInt32, alpha: Bool = false) throws -> Data {
  let context = try #require(CGContext(
    data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: alpha ? CGImageAlphaInfo.premultipliedLast.rawValue : CGImageAlphaInfo.noneSkipLast.rawValue
  ))
  if !alpha {
    context.setFillColor(CGColor(red: 1, green: 0, blue: 0, alpha: 1))
    context.fill(CGRect(x: 0, y: 0, width: width / 2, height: height))
    context.setFillColor(CGColor(red: 0, green: 0, blue: 1, alpha: 1))
    context.fill(CGRect(x: width / 2, y: 0, width: width - width / 2, height: height))
  }
  let data = NSMutableData()
  let type = alpha ? UTType.png : UTType.jpeg
  let destination = try #require(CGImageDestinationCreateWithData(data, type.identifier as CFString, 1, nil))
  CGImageDestinationAddImage(destination, try #require(context.makeImage()), [kCGImagePropertyOrientation: orientation] as CFDictionary)
  #expect(CGImageDestinationFinalize(destination))
  return data as Data
}

private func grayJpeg(_ width: Int, _ height: Int) throws -> Data {
  let context = try #require(CGContext(
    data: nil, width: width, height: height, bitsPerComponent: 8, bytesPerRow: 0,
    space: CGColorSpaceCreateDeviceGray(), bitmapInfo: CGImageAlphaInfo.none.rawValue
  ))
  let data = NSMutableData()
  let destination = try #require(CGImageDestinationCreateWithData(data, UTType.jpeg.identifier as CFString, 1, nil))
  CGImageDestinationAddImage(destination, try #require(context.makeImage()), nil)
  #expect(CGImageDestinationFinalize(destination))
  return data as Data
}

private func file(_ data: Data, _ fileExtension: String = "dat") throws -> URL {
  let url = FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).\(fileExtension)")
  try data.write(to: url)
  return url
}

private func image(_ data: Data) throws -> CGImage {
  let source = try #require(CGImageSourceCreateWithData(data as CFData, nil))
  return try #require(CGImageSourceCreateImageAtIndex(source, 0, nil))
}

private func rgb(_ image: CGImage, _ x: Int, _ y: Int) throws -> (r: Int, g: Int, b: Int, a: Int) {
  var bytes = [UInt8](repeating: 0, count: image.width * image.height * 4)
  let context = try #require(CGContext(
    data: &bytes, width: image.width, height: image.height, bitsPerComponent: 8, bytesPerRow: image.width * 4,
    space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
  ))
  context.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
  let at = (y * image.width + x) * 4
  return (Int(bytes[at]), Int(bytes[at + 1]), Int(bytes[at + 2]), Int(bytes[at + 3]))
}

private final class Ledger: @unchecked Sendable {
  private let lock = NSLock()
  private var items: [UInt64] = []
  private var releases = 0
  var asked: [UInt64] { lock.withLock { items } }
  var released: Int { lock.withLock { releases } }
  var admit: ImagePreparation.Admit {
    { bytes in
      self.lock.withLock { self.items.append(bytes) }
      return { self.lock.withLock { self.releases += 1 } }
    }
  }
}

private func edit(maxSize: PixelSize? = nil, enlarge: Bool = false, crop: CropFractions? = nil, outputSize: PixelSize? = nil, mirror: Bool = false, rotation: UInt32 = 0) -> PhotoEdit {
  PhotoEdit(maxSize: maxSize, enlarge: enlarge, crop: crop, outputSize: outputSize, mirror: mirror, rotationDegrees: rotation)
}

private func isRed(_ pixel: (r: Int, g: Int, b: Int, a: Int)) -> Bool { pixel.r > 200 && pixel.g < 60 && pixel.b < 60 }
private func isBlue(_ pixel: (r: Int, g: Int, b: Int, a: Int)) -> Bool { pixel.b > 200 && pixel.r < 60 && pixel.g < 60 }

@Suite(.serialized, .timeLimit(.minutes(1)))
struct MediaImageEditTests {
  @Test
  func inv_send_29_a_png_has_the_png_signature_and_a_jpeg_the_jpeg_one() async throws {
    let source = try image(try halves(80, 40, orientation: 1))
    let ledger = Ledger()

    let png = try await ImageEncoding.run(source, exifOrientation: 1, format: .png, quality: 0.9, admit: ledger.admit, sink: { try ImagePreparation.encoded($0, format: .png, quality: 0.9) })
    let jpeg = try await ImageEncoding.run(source, exifOrientation: 1, format: .jpeg, quality: 0.9, admit: ledger.admit, sink: { try ImagePreparation.encoded($0, format: .jpeg, quality: 0.9) })

    #expect(png.prefix(8) == Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]))
    #expect(jpeg.prefix(3) == Data([0xFF, 0xD8, 0xFF]))
    let raster: UInt64 = 80 * 40 * 4
    #expect(ledger.asked == [raster, raster])
    #expect(ledger.released == 2)
  }

  @Test
  func inv_send_30_an_encoded_image_is_upright_and_a_jpeg_flattens_transparency_onto_white() async throws {
    let sideways = try image(try halves(80, 40, orientation: 1))
    let transparent = try image(try halves(8, 8, orientation: 1, alpha: true))
    let ledger = Ledger()

    let turned = try image(try await ImageEncoding.run(sideways, exifOrientation: 6, format: .png, quality: 0.9, admit: ledger.admit, sink: { try ImagePreparation.encoded($0, format: .png, quality: 0.9) }))
    let flat = try image(try await ImageEncoding.run(transparent, exifOrientation: 1, format: .jpeg, quality: 0.9, admit: ledger.admit, sink: { try ImagePreparation.encoded($0, format: .jpeg, quality: 0.9) }))

    #expect((turned.width, turned.height) == (40, 80))
    #expect(isRed(try rgb(turned, 20, 20)))
    let corner = try rgb(flat, 4, 4)
    #expect(corner.r > 240 && corner.g > 240 && corner.b > 240)
    let costs: [UInt64] = [80 * 40 * 4 * 2, 8 * 8 * 4 * 2]
    #expect(ledger.asked == costs)
  }

  @Test
  func inv_send_30_a_refused_admission_stops_the_encode() async throws {
    let source = try image(try halves(80, 40, orientation: 1))

    await #expect(throws: MediaError.self) {
      try await ImageEncoding.run(source, exifOrientation: 1, format: .png, quality: 0.9, admit: { _ in throw MediaError.Overloaded(detail: "no room") }, sink: { _ in () })
    }
  }

  @Test
  func inv_send_31_the_size_of_bytes_equals_the_size_of_the_decode_without_decoding() async throws {
    let turned = try halves(80, 40, orientation: 6)
    let plain = try halves(80, 40, orientation: 1)

    let decoded = try await MediaImageDecode.decode(maxSide: nil, circle: false, admit: { _ in {} }) { CGImageSourceCreateWithData(turned as CFData, nil) }
    let size = try #require(MediaImageDecode.imageSize(data: turned))

    let pixels = try #require(decoded.cgImage)
    #expect((Int(size.width), Int(size.height)) == (pixels.width, pixels.height))
    #expect((size.width, size.height) == (40, 80))
    #expect(MediaImageDecode.imageSize(data: plain).map { ($0.width, $0.height) } ?? (0, 0) == (80, 40))
    #expect(MediaImageDecode.imageSize(data: Data([1, 2, 3])) == nil)
  }

  @Test
  func inv_send_43_an_encode_without_a_layer_gives_png_and_jpeg_bytes_turned_upright() async throws {
    let sideways = try halves(80, 40, orientation: 6)

    let png = try await MediaImageEncode.encode(bytes: sideways, format: .png)
    let jpeg = try await MediaImageEncode.encode(bytes: sideways, format: .jpeg, quality: 0.8)

    #expect(png.prefix(8) == Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]))
    #expect(jpeg.prefix(3) == Data([0xFF, 0xD8, 0xFF]))
    let turned = try image(png)
    #expect((turned.width, turned.height) == (40, 80))
    #expect(isRed(try rgb(turned, 20, 20)))
  }

  @Test
  func inv_send_44_an_encode_without_a_layer_refuses_a_raster_above_the_limit_with_a_media_error() async throws {
    let small = try halves(80, 40, orientation: 1)
    let large = try halves(2000, 2000, orientation: 6)

    await #expect(throws: MediaError.self) { try await MediaImageEncode.encode(bytes: small, limitBytes: 80 * 40 * 4 - 1) }
    await #expect(throws: MediaError.self) { try await MediaImageEncode.encode(bytes: large) }
    await #expect(throws: MediaError.self) { try await MediaImageEncode.encode(bytes: Data([1, 2, 3])) }
    #expect(try await MediaImageEncode.encode(bytes: small, limitBytes: 80 * 40 * 4).prefix(3) == Data([0xFF, 0xD8, 0xFF]))
  }
}

extension GlobalMedia {
  @Suite(.serialized, .timeLimit(.minutes(1)))
  struct MediaImageEncodeTests {
    private func open() async throws -> MediaLayer {
      try await MediaLayer.open(
        fetcher: TestFetcher(),
        settings: UserDefaultsSettings(defaults: UserDefaults(suiteName: UUID().uuidString)!),
        directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
      )
    }

    @Test
    func inv_send_43_an_encode_without_a_layer_equals_the_encode_through_the_layer() async throws {
      let media = try await open()
      let webp = try Data(contentsOf: Bundle.module.url(forResource: "sticker", withExtension: "webp", subdirectory: "Fixtures/alpha")!)
      let jpeg = try halves(80, 40, orientation: 6)

      for (bytes, format) in [(webp, ImageFormat.png), (jpeg, .jpeg), (webp, .jpeg)] {
        let outside = try await MediaImageEncode.encode(bytes: bytes, format: format, quality: 0.7)
        let inside = try await media.encodeImage(bytes: bytes, format: format, quality: 0.7)
        #expect(outside == inside)
      }
    }

    @Test
    func inv_send_32_a_webp_becomes_png_bytes_and_an_image_in_memory_becomes_a_file_through_the_layer() async throws {
      let media = try await open()
      let webp = try Data(contentsOf: Bundle.module.url(forResource: "sticker", withExtension: "webp", subdirectory: "Fixtures/alpha")!)

      let png = try await media.encodeImage(bytes: webp, format: .png)
      let prepared = try await media.encodeImage(try image(png), format: .jpeg, quality: 0.8)
      defer { try? FileManager.default.removeItem(at: prepared.url) }

      #expect(png.prefix(8) == Data([0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]))
      #expect(prepared.isNew)
      #expect(try Data(contentsOf: prepared.url).prefix(3) == Data([0xFF, 0xD8, 0xFF]))
      await #expect(throws: MediaError.self) { try await media.encodeImage(bytes: Data([1, 2, 3]), format: .png) }

      let sideways = UIImage(cgImage: try image(try halves(80, 40, orientation: 1)), scale: 1, orientation: .right)
      let upright = try await media.encodeImage(sideways, format: .png)
      defer { try? FileManager.default.removeItem(at: upright.url) }
      let turned = try image(Data(contentsOf: upright.url))
      #expect((turned.width, turned.height) == (40, 80))
    }

    @Test
    func inv_send_45_the_encoded_photo_file_holds_the_bytes_of_the_data_encode() async throws {
      let media = try await open()
      let picture = try image(try halves(80, 40, orientation: 1))

      for (format, type) in [(ImageFormat.png, UTType.png), (.jpeg, .jpeg)] {
        let expected = NSMutableData()
        let destination = try #require(CGImageDestinationCreateWithData(expected, type.identifier as CFString, 1, nil))
        let options: CFDictionary? = format == .png ? nil : [kCGImageDestinationLossyCompressionQuality: jpegQuality(quality: 0.7)] as CFDictionary
        CGImageDestinationAddImage(destination, picture, options)
        #expect(CGImageDestinationFinalize(destination))

        let prepared = try await media.encodeImage(picture, format: format, quality: 0.7)
        defer { try? FileManager.default.removeItem(at: prepared.url) }

        #expect(prepared.isNew)
        #expect(prepared.url.pathExtension == (format == .png ? "png" : "jpg"))
        #expect(try Data(contentsOf: prepared.url) == expected as Data)
      }
    }

  }
}
