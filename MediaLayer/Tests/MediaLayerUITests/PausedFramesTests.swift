#if canImport(UIKit)
import ImageIO
import SwiftUI
import Testing
import UIKit
import UniformTypeIdentifiers
@testable import MediaLayerUI

private func decodedFrames(byteBudget: Int = AnimatedFrames.byteBudget) throws -> AnimatedFrames {
  let data = NSMutableData()
  let destination = try #require(CGImageDestinationCreateWithData(data, UTType.gif.identifier as CFString, 3, nil))
  for index in 0 ..< 3 {
    let context = try #require(CGContext(
      data: nil, width: 16, height: 16, bitsPerComponent: 8, bytesPerRow: 0,
      space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
    ))
    context.setFillColor(CGColor(red: 1, green: 0, blue: 0, alpha: 1))
    context.fill(CGRect(x: index, y: 0, width: 8, height: 8))
    CGImageDestinationAddImage(destination, try #require(context.makeImage()), [kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFDelayTime: 0.1]] as CFDictionary)
  }
  CGImageDestinationFinalize(destination)
  return try #require(AnimatedFrames.decode(data as Data, maxPixel: 16, byteBudget: byteBudget))
}

@MainActor
private func attach(_ view: UIView) -> UIWindow {
  let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 100, height: 100))
  window.addSubview(view)
  return window
}

@MainActor
private func update(_ view: UIImageView, _ playback: AnimatedFramesView.Playback, _ frames: AnimatedFrames, playing: Bool) {
  AnimatedFramesView(frames: frames, contentMode: .fill, isPlaying: playing, playbackRate: 1).update(view, playback: playback)
}

@MainActor
@Suite(.timeLimit(.minutes(1)))
struct PausedFramesTests {
  @Test
  func inv_idle_01_streamed_gif_owns_one_preparation_until_cancelled() async throws {
    let frames = try decodedFrames(byteBudget: 1)
    guard case let .streamed(stream) = frames.storage else { Issue.record("expected streamed frames"); return }
    let view = FrameImageView()
    let playback = AnimatedFramesView.Playback()
    let window = attach(view)
    update(view, playback, frames, playing: true)
    try await Task.sleep(for: .milliseconds(450))
    #expect(stream.decodes >= 2)
    #expect(stream.preparations == 1)
    update(view, playback, frames, playing: false)
    try await Task.sleep(for: .milliseconds(50))
    let stopped = stream.decodes
    try await Task.sleep(for: .milliseconds(250))
    #expect(stream.decodes == stopped)
    withExtendedLifetime(window) {}
  }

  @Test
  func a_paused_decoded_gif_still_draws_its_first_frame() throws {
    let frames = try decodedFrames()
    let view = UIImageView()
    let playback = AnimatedFramesView.Playback()

    update(view, playback, frames, playing: false)

    #expect(view.image != nil, "paused: isAnimating=\(view.isAnimating) image=\(String(describing: view.image))")
  }

  @Test
  func a_gif_that_left_and_came_back_animates_again_and_keeps_a_frame_while_away() throws {
    let frames = try decodedFrames()
    let view = FrameImageView()
    let window = attach(view)
    let playback = AnimatedFramesView.Playback()

    update(view, playback, frames, playing: true)
    #expect(view.isAnimating)
    update(view, playback, frames, playing: false)
    #expect(!view.isAnimating)
    #expect(view.image != nil, "away: image=\(String(describing: view.image))")
    update(view, playback, frames, playing: true)
    #expect(view.isAnimating)
    withExtendedLifetime(window) {}
  }

  @Test
  func a_decoded_gif_animates_only_while_its_view_is_in_a_window() throws {
    let frames = try decodedFrames()
    let view = FrameImageView()
    let playback = AnimatedFramesView.Playback()

    update(view, playback, frames, playing: true)
    #expect(!view.isAnimating)
    let window = attach(view)
    #expect(view.isAnimating)
    view.removeFromSuperview()
    #expect(!view.isAnimating)
    window.addSubview(view)
    #expect(view.isAnimating)
  }
}
#endif
