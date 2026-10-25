#if canImport(UIKit)
import CoreGraphics
import ImageIO
import MediaLayer
import Nuke
import SwiftUI
import Testing
import UIKit
import UniformTypeIdentifiers
@testable import MediaLayerUI

private let animatedWebP = Data(base64Encoded: "UklGRrQAAABXRUJQVlA4WAoAAAACAAAADwAADwAAQU5JTQYAAAAAAAAAAABBTk1GKAAAAAAAAAAAAAcAAAcAAGQAAANWUDhMDwAAAC8HwAEABxD9j/4HIqL/AQBBTk1GKAAAAAIAAAAAAAcAAAcAAGQAAAFWUDhMDwAAAC8HwAEABxD9j/4HIqL/AQBBTk1GKAAAAAQAAAAAAAcAAAcAAGQAAABWUDhMDwAAAC8HwAEABxD9j/4HIqL/AQA=")!

private func stepGif() throws -> AnimatedFrames {
  try #require(AnimatedFrames.decode(try gifData(), maxPixel: 16))
}

private func signature(_ image: CGImage?) -> [UInt8] {
  guard let image, let context = CGContext(
    data: nil, width: 16, height: 16, bitsPerComponent: 8, bytesPerRow: 64,
    space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
  ) else { return [] }

  context.draw(image, in: CGRect(x: 0, y: 0, width: 16, height: 16))
  guard let data = context.data else { return [] }

  return Array(UnsafeBufferPointer(start: data.assumingMemoryBound(to: UInt8.self), count: 16 * 64))
}

private func nearest(_ image: CGImage?, to references: [[UInt8]]) -> Int? {
  let shown = signature(image)
  guard shown.count == references.first?.count else { return nil }

  let distances = references.map { reference in zip(shown, reference).reduce(0) { $0 + abs(Int($1.0) - Int($1.1)) } }
  return distances.enumerated().min { $0.element < $1.element }?.offset
}

@MainActor
private func attached(_ view: UIView) -> UIWindow {
  let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 100, height: 100))
  window.addSubview(view)
  return window
}

@MainActor
private func update(_ view: UIImageView, _ playback: AnimatedFramesView.Playback, _ frames: AnimatedFrames, animation: ImageAnimation = ImageAnimation(), rests: Bool = false) {
  AnimatedFramesView(frames: frames, contentMode: .fill, isPlaying: true, playbackRate: 1, animation: animation, rests: rests).update(view, playback: playback)
}

@MainActor
private func frameView(in view: UIView) -> FrameImageView? {
  if let found = view as? FrameImageView { return found }
  for child in view.subviews { if let found = frameView(in: child) { return found } }
  return nil
}

private struct AnimatedCell: MediaSource {
  let url: String
  var kind: Kind { .image }
  var headers: [String: String] { [:] }
  var size: MediaSize? { MediaSize(width: 16, height: 16) }
}

private struct MotionCell: View {
  let url: String
  let context: MediaContext
  let reduceMotion: Bool
  var animation = ImageAnimation()

  var body: some View {
    MediaImage(AnimatedCell(url: url), animation: animation)
      .environment(\.mediaContext, context)
      .environment(\.mediaReduceMotionOverride, reduceMotion)
      .frame(width: 50, height: 50)
  }
}

extension GlobalMedia {
  @MainActor
  @Suite(.timeLimit(.minutes(2)))
  struct ImageAnimationTests {
    @Test
    func inv_image_51_a_resting_animation_shows_its_first_frame_starts_no_playback_and_plays_again_when_released() async throws {
      let frames = try stepGif()
      let view = FrameImageView()
      let playback = AnimatedFramesView.Playback()
      let window = attached(view)

      update(view, playback, frames, rests: true)
      #expect(signature(view.image?.cgImage) == signature(frames.firstImage))
      #expect(frames.stream.preparations == 0)
      #expect(frames.stream.decodes == 0)

      update(view, playback, frames, rests: false)
      #expect(frames.stream.preparations == 1)
      await eventually("the released animation decoded frames") { frames.stream.decodes > 0 }
      withExtendedLifetime(window) {}
    }

    @Test(arguments: [ImageAnimation.RestFrame.first, .last])
    func inv_image_52_an_animation_with_repeats_stops_after_them_on_its_rest_frame(rest: ImageAnimation.RestFrame) async throws {
      let frames = try stepGif()
      let view = FrameImageView()
      let playback = AnimatedFramesView.Playback()
      let window = attached(view)
      let animation = ImageAnimation(repeats: .times(2), rest: rest)

      update(view, playback, frames, animation: animation)
      await eventually("the repeats ended") { playback.finished != nil }
      let expected = signature(frames.image(at: rest == .last ? 2 : 0))
      #expect(signature(view.image?.cgImage) == expected)

      let shown = view.framesShown
      update(view, playback, frames, animation: animation)
      #expect(frames.stream.preparations == 1)
      #expect(view.framesShown == shown)
      withExtendedLifetime(window) {}
    }

    @Test
    func inv_image_53_a_resting_animation_shows_the_frame_it_rests_on() async throws {
      let frames = try stepGif()
      let view = FrameImageView()
      let playback = AnimatedFramesView.Playback()
      let window = attached(view)

      update(view, playback, frames, animation: ImageAnimation(rest: .last), rests: true)
      await eventually("the last frame is shown") { signature(view.image?.cgImage) == signature(frames.image(at: 2)) }
      #expect(signature(view.image?.cgImage) != signature(frames.firstImage))
      #expect(frames.stream.preparations == 0)

      update(view, playback, frames, animation: ImageAnimation(rest: .first), rests: true)
      #expect(signature(view.image?.cgImage) == signature(frames.firstImage))
      withExtendedLifetime(window) {}
    }

    @Test
    func inv_image_54_the_system_setting_rests_the_animation_unless_the_app_allows_it() {
      #expect(ImageAnimation().rests(reduceMotion: false) == false)
      #expect(ImageAnimation().rests(reduceMotion: true))
      #expect(ImageAnimation(ignoresReduceMotion: true).rests(reduceMotion: true) == false)
      #expect(ImageAnimation(playing: false).rests(reduceMotion: false))
      #expect(ImageAnimation(playing: false, ignoresReduceMotion: true).rests(reduceMotion: false))
    }

    @Test
    func inv_image_54_the_reduce_motion_setting_rests_a_shown_animation_and_lifting_it_plays_again() async throws {
      let places = RecordingPlaces()
      places.imageResult = .success(try gifData())
      ImageMemory.install(ImageCache(), on: places)
      let context = MediaContext(places: places)
      let url = "https://o.test/\(UUID().uuidString).gif"
      let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 100, height: 100))
      let host = UIHostingController(rootView: MotionCell(url: url, context: context, reduceMotion: false))
      window.rootViewController = host
      window.isHidden = false
      defer { window.isHidden = true; window.rootViewController = nil }

      await eventually("the animation moves") { (frameView(in: host.view)?.framesShown ?? 0) > 3 }
      host.rootView = MotionCell(url: url, context: context, reduceMotion: true)
      let references = try (0 ..< 3).map { signature(try stepGif().image(at: $0)) }
      await eventually("the animation rests on its first frame") { nearest(frameView(in: host.view)?.image?.cgImage, to: references) == 0 }
      try? await Task.sleep(for: .milliseconds(350))
      let resting = try #require(frameView(in: host.view)).framesShown
      try? await Task.sleep(for: .milliseconds(350))
      #expect(frameView(in: host.view)?.framesShown == resting, "the resting animation kept changing frames")
      host.rootView = MotionCell(url: url, context: context, reduceMotion: false)
      await eventually("the animation plays again") { (frameView(in: host.view)?.framesShown ?? 0) > resting + 1 }
    }

    @Test
    func inv_image_55_an_animated_webp_decodes_to_frames_with_their_delays_and_plays_through_the_pipeline() async throws {
      let frames = try #require(AnimatedFrames.decode(animatedWebP, maxPixel: 16))
      #expect(frames.frameCount == 3)
      #expect(abs(frames.duration - 0.3) < 0.001)

      let places = RecordingPlaces()
      places.imageResult = .success(animatedWebP)
      ImageMemory.install(ImageCache(), on: places)
      let context = MediaContext(places: places)
      let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 100, height: 100))
      let host = UIHostingController(rootView: MotionCell(url: "https://o.test/\(UUID().uuidString).webp", context: context, reduceMotion: false))
      window.rootViewController = host
      window.isHidden = false
      defer { window.isHidden = true; window.rootViewController = nil }

      await eventually("the webp moves") { (frameView(in: host.view)?.framesShown ?? 0) > 3 }
    }
  }
}

private func gifData() throws -> Data {
  let data = NSMutableData()
  let destination = try #require(CGImageDestinationCreateWithData(data, UTType.gif.identifier as CFString, 3, nil))
  for index in 0 ..< 3 {
    let context = try #require(CGContext(
      data: nil, width: 16, height: 16, bitsPerComponent: 8, bytesPerRow: 0,
      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ))
    context.setFillColor(CGColor(red: 1, green: 0, blue: 0, alpha: 1))
    context.fill(CGRect(x: index * 4, y: 0, width: 8, height: 8))
    CGImageDestinationAddImage(destination, try #require(context.makeImage()), [kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFDelayTime: 0.1]] as CFDictionary)
  }
  CGImageDestinationFinalize(destination)
  return data as Data
}
#endif
