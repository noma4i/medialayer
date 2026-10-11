import CoreGraphics
import Foundation
import ImageIO
import MediaLayer
import Nuke
import Testing
import UniformTypeIdentifiers
@testable import MediaLayerUI

private struct GifSource: MediaSource {
  let url: String
  var kind: Kind { .image }
  var headers: [String: String] = [:]
  var size: MediaSize?
}

private func transparentGif(frames: Int, side: Int, delay: Double = 0.1) -> Data {
  let data = NSMutableData()
  let destination = CGImageDestinationCreateWithData(data, UTType.gif.identifier as CFString, frames, nil)!
  CGImageDestinationSetProperties(destination, [kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFLoopCount: 0]] as CFDictionary)
  for index in 0 ..< frames {
    let context = CGContext(
      data: nil, width: side, height: side, bitsPerComponent: 8, bytesPerRow: 0,
      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    )!
    context.setFillColor(CGColor(red: 1, green: 0, blue: 0, alpha: 1))
    context.fill(CGRect(x: side / 4 + index, y: side / 4, width: side / 2, height: side / 2))
    CGImageDestinationAddImage(destination, context.makeImage()!, [kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFDelayTime: delay]] as CFDictionary)
  }
  CGImageDestinationFinalize(destination)
  return data as Data
}

private func alpha(of image: CGImage, x: Int, y: Int) -> UInt8 {
  var pixel = [UInt8](repeating: 0, count: 4)
  pixel.withUnsafeMutableBytes { bytes in
    let context = CGContext(
      data: bytes.baseAddress, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    )!
    context.draw(image, in: CGRect(x: -x, y: -(image.height - 1 - y), width: image.width, height: image.height))
  }
  return pixel[3]
}

struct AnimatedFramesTests {
  @Test
  func inv_image_06_every_frame_of_a_gif_keeps_its_transparency_and_the_loop_runs_for_the_sum_of_the_delays() throws {
    let frames = try #require(AnimatedFrames.decode(transparentGif(frames: 3, side: 16), maxPixel: 16))

    #expect(frames.images.count == 3)
    #expect(abs(frames.duration - 0.3) < 0.001)
    for image in frames.images {
      #expect(alpha(of: image, x: 0, y: 0) == 0)
      #expect(alpha(of: image, x: 8, y: 8) == 255)
    }
    #expect(AnimatedFrames.decode(pngData(side: 16), maxPixel: 16) == nil)
  }

  @Test
  func inv_image_06_a_gif_loaded_through_the_pipeline_hands_its_original_bytes_to_the_playback() async throws {
    let places = RecordingPlaces()
    places.imageResult = .success(transparentGif(frames: 3, side: 16))
    ImageMemory.install(ImageCache(), on: places)
    let pipeline = MediaImagePipeline.make(places: places)
    let request = try #require(MediaImagePipeline.request(for: GifSource(url: "https://o.test/a.gif"), points: CGSize(width: 8, height: 8), scale: 1))

    let container = try await pipeline.imageTask(with: request).response.container
    let data = try #require(container.data)

    #expect(container.type == .gif)
    #expect(AnimatedFrames.decode(data, maxPixel: 8)?.images.count == 3)
  }

  @Test
  func inv_image_06_a_playback_rate_stretches_the_loop_of_a_gif_stamped_faster_than_its_video() throws {
    let frames = try #require(AnimatedFrames.decode(transparentGif(frames: 40, side: 16, delay: 0.05), maxPixel: 16))

    #expect(abs(frames.loopDuration(rate: 1) - 2) < 0.001)
    #expect(abs(frames.loopDuration(rate: 0.5) - 4) < 0.001)
    #expect(abs(frames.loopDuration(rate: 0) - 2) < 0.001)
  }
}
