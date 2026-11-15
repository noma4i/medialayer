import AVFoundation
import CoreGraphics
import CryptoKit
import Foundation
import Testing
@testable import MediaLayerUI

@Suite(.timeLimit(.minutes(1)))
struct UprightVideoTests {
  private func copy(_ name: String) throws -> URL {
    let source = try #require(Bundle.module.url(forResource: name, withExtension: "mp4", subdirectory: "Fixtures"))
    let target = FileManager.default.temporaryDirectory.appendingPathComponent("upright-\(UUID().uuidString).mp4")
    try FileManager.default.copyItem(at: source, to: target)
    return target
  }

  private func frame(_ url: URL, upright: Bool) async throws -> CGImage {
    let generator = AVAssetImageGenerator(asset: AVURLAsset(url: url))
    generator.appliesPreferredTrackTransform = upright
    generator.requestedTimeToleranceBefore = .zero
    generator.requestedTimeToleranceAfter = .zero
    return try await generator.image(at: CMTime(value: 1, timescale: 2)).image
  }

  private func quadrants(_ image: CGImage) throws -> [[Int]] {
    let width = image.width
    let height = image.height
    var bytes = [UInt8](repeating: 0, count: width * height * 4)
    let context = try #require(CGContext(data: &bytes, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
    context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
    return [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)].map { point in
      let index = (Int(Double(height) * point.1) * width + Int(Double(width) * point.0)) * 4
      return [Int(bytes[index]), Int(bytes[index + 1]), Int(bytes[index + 2])]
    }
  }

  @Test
  func inv_capture_29_a_recording_with_a_turned_track_comes_out_upright_in_its_pixels() async throws {
    let url = try copy("rotated-recording")
    defer { try? FileManager.default.removeItem(at: url) }
    let shownImage = try await frame(url, upright: true)
    #expect(shownImage.width == 180 && shownImage.height == 320)
    let shown = try quadrants(shownImage)

    try await uprightVideo(at: url)

    let asset = AVURLAsset(url: url)
    let video = try #require(try await asset.loadTracks(withMediaType: .video).first)
    let (size, transform) = try await video.load(.naturalSize, .preferredTransform)
    #expect(transform == .identity)
    #expect(size == CGSize(width: 180, height: 320))
    #expect(try await asset.loadTracks(withMediaType: .audio).count == 1)
    #expect(abs(try await asset.load(.duration).seconds - 1) < 0.1)
    let stored = try quadrants(try await frame(url, upright: false))
    #expect(zip(stored, shown).allSatisfy { pair in zip(pair.0, pair.1).allSatisfy { abs($0 - $1) < 48 } }, "stored \(stored) shown \(shown)")
  }

  @Test
  func inv_capture_29_an_upright_recording_is_left_as_it_is() async throws {
    let url = try copy("silent")
    defer { try? FileManager.default.removeItem(at: url) }
    let before = SHA256.hash(data: try Data(contentsOf: url))

    try await uprightVideo(at: url)

    #expect(SHA256.hash(data: try Data(contentsOf: url)) == before)
  }
}
