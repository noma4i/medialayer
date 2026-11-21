import CoreGraphics
import Darwin
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

private func transparentGif(frames: Int, side: Int, delay: Double = 0.1, delays: [Double]? = nil) -> Data {
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
    CGImageDestinationAddImage(destination, context.makeImage()!, [kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFDelayTime: delays?[index] ?? delay]] as CFDictionary)
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

private func gifOfSharedFrames(frames: Int, side: Int) -> Data {
  let variants = (0 ..< 8).map { _ -> CGImage in
    let context = CGContext(
      data: nil, width: side, height: side, bitsPerComponent: 8, bytesPerRow: 0,
      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue
    )!
    arc4random_buf(context.data, context.bytesPerRow * side)
    return context.makeImage()!
  }
  let data = NSMutableData()
  let destination = CGImageDestinationCreateWithData(data, UTType.gif.identifier as CFString, frames, nil)!
  for index in 0 ..< frames {
    CGImageDestinationAddImage(destination, variants[index % variants.count], [kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFDelayTime: 0.1]] as CFDictionary)
  }
  CGImageDestinationFinalize(destination)
  return data as Data
}

struct AnimatedFramesTests {
  @Test
  func inv_image_31_gif_uses_each_frame_delay() throws {
    let frames = try #require(AnimatedFrames.decode(transparentGif(frames: 3, side: 16, delays: [0.1, 0.3, 0.2]), maxPixel: 16))
    for (elapsed, expected) in [(0.05, 0), (0.15, 1), (0.35, 1), (0.45, 2), (0.65, 3)] {
      #expect(frames.frameTick(elapsed: elapsed, rate: 1) == expected)
    }
  }

  @Test
  func inv_image_32_small_gif_keeps_a_bounded_decoded_window() throws {
    let frames = try #require(AnimatedFrames.decode(transparentGif(frames: 40, side: 16), maxPixel: 16))
    let stream = frames.stream
    for index in 0 ..< frames.frameCount {
      #expect(frames.image(at: index) != nil)
      #expect(stream.cachedCount <= FrameStream.window)
    }
  }

  @Test
  func inv_image_06_every_frame_of_a_gif_keeps_its_transparency_and_the_loop_runs_for_the_sum_of_the_delays() throws {
    let frames = try #require(AnimatedFrames.decode(transparentGif(frames: 3, side: 16), maxPixel: 16))

    #expect(frames.frameCount == 3)
    #expect(abs(frames.duration - 0.3) < 0.001)
    for image in (0 ..< frames.frameCount).compactMap(frames.image(at:)) {
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
    let request = try #require(MediaImagePipeline.request(places: places, for: GifSource(url: "https://o.test/a.gif"), points: CGSize(width: 8, height: 8), scale: 1))

    let container = try await pipeline.imageTask(with: request).response.container
    let data = try #require(container.data)

    #expect(container.type == .gif)
    #expect(AnimatedFrames.decode(data, maxPixel: 8)?.frameCount == 3)
  }

  @Test
  func inv_image_06_a_playback_rate_stretches_the_loop_of_a_gif_stamped_faster_than_its_video() throws {
    let frames = try #require(AnimatedFrames.decode(transparentGif(frames: 40, side: 16, delay: 0.05), maxPixel: 16))

    #expect(abs(frames.loopDuration(rate: 1) - 2) < 0.001)
    #expect(abs(frames.loopDuration(rate: 0.5) - 4) < 0.001)
    #expect(abs(frames.loopDuration(rate: 0) - 2) < 0.001)
  }

  @Test
  func inv_image_06_a_cancelled_decode_stops_between_frames_and_returns_nothing() async {
    let data = transparentGif(frames: 60, side: 16)
    let task = Task { () -> AnimatedFrames? in
      try? await Task.sleep(for: .seconds(30))
      return await AnimatedFrames.load(data, maxPixel: 16)
    }
    task.cancel()

    #expect(await task.value == nil)
    var checks = 0
    let stopped = AnimatedFrames.decode(data, maxPixel: 16, isCancelled: { checks += 1; return checks > 3 })
    #expect(stopped == nil)
    #expect(checks == 4)
  }

  @Test
  func aGifOverTheBudgetPeaksBelowTheBudgetWhileDecoding() throws {
    let data = gifOfSharedFrames(frames: 200, side: 600)
    let frames = try #require(AnimatedFrames.decode(data, maxPixel: 600))

    #expect(frames.retainedBytes <= 32 << 20)
    #expect(frames.stream.decodes == 0)
    #expect(frames.stream.cachedCount == 0)
    #expect(frames.frameCount == 200)
  }

  @Test
  func inv_image_87_a_non_finite_or_non_positive_playback_rate_plays_at_normal_speed() throws {
    let frames = try #require(AnimatedFrames.decode(transparentGif(frames: 40, side: 16, delay: 0.05), maxPixel: 16))
    for rate in [Double.infinity, -Double.infinity, .nan, 0, -2] {
      #expect(playbackRateOf(rate) == 1)
      #expect(frames.frameTick(elapsed: 1.01, rate: rate) == frames.frameTick(elapsed: 1.01, rate: 1))
      #expect(frames.loopDuration(rate: rate) == frames.duration)
    }
    #expect(playbackRateOf(2) == 2)
  }

  @Test
  func aStreamedGifShowsTheFrameOfTheElapsedTimeSoALateWakeNeverStretchesTheLoop() throws {
    let frames = try #require(AnimatedFrames.decode(transparentGif(frames: 40, side: 16, delay: 0.05), maxPixel: 16))

    #expect(frames.frameTick(elapsed: 0.99, rate: 1) == 19)
    #expect(frames.frameTick(elapsed: 1.01, rate: 1) == 20)
    #expect(frames.frameTick(elapsed: 2.06, rate: 1) % frames.frameCount == 1)
    #expect(frames.frameTick(elapsed: 1.01, rate: 0.5) == 10)
  }
}
