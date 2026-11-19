#if os(iOS)
import AVFoundation
import CoreGraphics
import CoreVideo
import Foundation
import Testing
import UIKit
@testable import MediaLayerUI

// INV-CAPTURE-80: the recorded pixels stand as the preview showed them, in any pose of the device: no pose sensor takes part, the preview angle is the only source.
// INV-CAPTURE-29 only proves that a turned matrix becomes pixels; this guard proves the capture side gives the right turn.
@Suite(.timeLimit(.minutes(2)))
struct TakeTransformTests {
  private static let width = 320
  private static let height = 240

  private func sensorFrame(upEdge: String) throws -> CVPixelBuffer {
    var pixels: CVPixelBuffer?
    try #require(CVPixelBufferCreate(nil, Self.width, Self.height, kCVPixelFormatType_32BGRA, nil, &pixels) == kCVReturnSuccess)
    let buffer = try #require(pixels)
    CVPixelBufferLockBaseAddress(buffer, [])
    defer { CVPixelBufferUnlockBaseAddress(buffer, []) }
    let base = try #require(CVPixelBufferGetBaseAddress(buffer)).assumingMemoryBound(to: UInt8.self)
    let stride = CVPixelBufferGetBytesPerRow(buffer)
    let band = 60
    for y in 0..<Self.height {
      for x in 0..<Self.width {
        let marked: Bool
        switch upEdge {
        case "left": marked = x < band
        case "right": marked = x >= Self.width - band
        case "top": marked = y < band
        default: marked = y >= Self.height - band
        }
        let at = y * stride + x * 4
        base[at] = marked ? 0 : 255
        base[at + 1] = marked ? 0 : 255
        base[at + 2] = marked ? 255 : 255
        base[at + 3] = 255
      }
    }
    return buffer
  }

  private func record(mirror: Bool, upEdge: String) async throws -> URL {
    let url = FileManager.default.temporaryDirectory.appendingPathComponent("capture-angle-\(UUID().uuidString).mp4")
    let writer = try AVAssetWriter(outputURL: url, fileType: .mp4)
    let input = AVAssetWriterInput(mediaType: .video, outputSettings: [AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: Self.width, AVVideoHeightKey: Self.height])
    input.transform = takeTransform(mirror: mirror)
    let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: input, sourcePixelBufferAttributes: nil)
    writer.add(input)
    try #require(writer.startWriting())
    writer.startSession(atSourceTime: .zero)
    let frame = try sensorFrame(upEdge: upEdge)
    for index in 0..<10 {
      while !input.isReadyForMoreMediaData { try await Task.sleep(nanoseconds: 2_000_000) }
      try #require(adaptor.append(frame, withPresentationTime: CMTime(value: CMTimeValue(index), timescale: 10)))
    }
    input.markAsFinished()
    await writer.finishWriting()
    try #require(writer.status == .completed)
    return url
  }

  private func topCenterIsMarked(_ url: URL) async throws -> Bool {
    let generator = AVAssetImageGenerator(asset: AVURLAsset(url: url))
    generator.appliesPreferredTrackTransform = true
    let image = try await generator.image(at: CMTime(value: 5, timescale: 10)).image
    let width = image.width
    let height = image.height
    var bytes = [UInt8](repeating: 0, count: width * height * 4)
    let context = try #require(CGContext(data: &bytes, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
    context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
    func red(_ x: Int, _ y: Int) -> Bool { bytes[(y * width + x) * 4 + 1] < 100 }
    // The band stands at the top of the picture and nowhere else along the middle column.
    return red(width / 2, height / 12) && !red(width / 2, height - height / 12)
  }

  @Test
  func inv_capture_80_the_recorded_pixels_stand_as_the_portrait_preview_front_and_back_mirrored_or_not() async throws {
    for mirror in [false, true] {
      let url = try await record(mirror: mirror, upEdge: "left")
      defer { try? FileManager.default.removeItem(at: url) }
      try await uprightVideo(at: url)
      let track = try #require(try await AVURLAsset(url: url).loadTracks(withMediaType: .video).first)
      #expect(try await track.load(.preferredTransform) == .identity, "mirror \(mirror)")
      #expect(try await topCenterIsMarked(url), "mirror \(mirror)")
    }
  }

  @Test
  func the_take_capture_angle_follows_the_lens_offset_against_the_back_camera() {
    #expect(takeCaptureAngle(lensCapture: 90, backCapture: 90) == 90)
    #expect(takeCaptureAngle(lensCapture: 0, backCapture: 90) == 0)
    #expect(takeCaptureAngle(lensCapture: 180, backCapture: 180) == 90)
    #expect(takeCaptureAngle(lensCapture: 270, backCapture: 0) == 0)
  }

  @Test
  func the_take_capture_angle_is_the_same_in_every_pose() {
    for offset: CGFloat in [-90, 0, 90, 180] {
      let angles = [CGFloat(0), 90, 180, 270].map { pose in
        takeCaptureAngle(lensCapture: (pose + offset + 360).truncatingRemainder(dividingBy: 360), backCapture: (pose + 90).truncatingRemainder(dividingBy: 360))
      }
      #expect(Set(angles).count == 1, "offset \(offset)")
    }
  }
}
#endif
