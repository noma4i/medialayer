import CoreGraphics
import Foundation
import ImageIO
import MediaLayer
import Testing
import UniformTypeIdentifiers
@testable import MediaLayerUI

struct ToneFileReadTests {
  @Test
  func inv_native_25_file_url_tone_goes_through_fetch_image() async throws {
    let context = try #require(CGContext(
      data: nil, width: 32, height: 32, bitsPerComponent: 8, bytesPerRow: 0,
      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ))
    context.setFillColor(CGColor(red: 0.2, green: 0.4, blue: 0.6, alpha: 1))
    context.fill(CGRect(x: 0, y: 0, width: 32, height: 32))
    let encoded = NSMutableData()
    let destination = try #require(CGImageDestinationCreateWithData(encoded, UTType.png.identifier as CFString, 1, nil))
    CGImageDestinationAddImage(destination, try #require(context.makeImage()), nil)
    #expect(CGImageDestinationFinalize(destination))
    let places = RecordingPlaces()
    places.needsTone = true
    places.imageResult = .success(encoded as Data)
    let url = "file:///tmp/\(UUID().uuidString).png"
    let item = MediaItem(kind: .image, url: url, headers: [:], cover: nil, blurHash: nil, strategy: .window)

    let pixels = await ToneFrames(places: places).prepare(source: item)

    #expect(places.fetchedURLs == [url])
    #expect(pixels?.rgba.count == 32 * 32 * 4)
  }
}
