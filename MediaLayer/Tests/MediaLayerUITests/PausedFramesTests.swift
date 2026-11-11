#if canImport(UIKit)
import ImageIO
import SwiftUI
import Testing
import UIKit
import UniformTypeIdentifiers
@testable import MediaLayerUI

func decodedFrames() throws -> AnimatedFrames {
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
  return try #require(AnimatedFrames.decode(data as Data, maxPixel: 16))
}

@MainActor
private func attach(_ view: UIView) -> UIWindow {
  let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 100, height: 100))
  window.addSubview(view)
  return window
}

@MainActor
private func update(_ view: FrameLayerView, _ playback: AnimatedFramesView.Playback, _ frames: AnimatedFrames, playing: Bool, grayscale: Bool = false) {
  AnimatedFramesView(frames: frames, contentMode: .fill, isPlaying: playing, playbackRate: 1, grayscale: grayscale).update(view, playback: playback)
}

enum PresentingStop: String, CaseIterable, CustomTestStringConvertible {
  case stopPresenting, leftWindow

  var testDescription: String { rawValue }
}

@MainActor
@Suite(.timeLimit(.minutes(1)))
struct PausedFramesTests {
  @Test
  func inv_idle_01_streamed_gif_owns_one_preparation_until_cancelled() async throws {
    var frames: AnimatedFrames? = try decodedFrames()
    let view = FrameLayerView()
    let playback = AnimatedFramesView.Playback()
    let window = attach(view)
    weak var stream = frames?.stream
    update(view, playback, try #require(frames), playing: true)
    await eventually("the stream decoded two frames") { stream?.decodes ?? 0 >= 2 }
    #expect(stream?.preparations == 1)
    update(view, playback, try #require(frames), playing: false)
    frames = nil
    await eventually("the cancelled playback task released the stream") { stream == nil }
    withExtendedLifetime(window) {}
  }

  @Test
  func a_paused_decoded_gif_still_draws_its_first_frame() throws {
    let frames = try decodedFrames()
    let view = FrameLayerView()
    let playback = AnimatedFramesView.Playback()

    update(view, playback, frames, playing: false)

    #expect(view.shownContents != nil)
  }

  @Test
  func a_gif_that_left_and_came_back_animates_again_and_keeps_a_frame_while_away() async throws {
    let frames = try decodedFrames()
    let view = FrameLayerView()
    let window = attach(view)
    let playback = AnimatedFramesView.Playback()

    update(view, playback, frames, playing: true)
    await eventually("the gif decoded a frame") { frames.stream.decodes > 0 }
    update(view, playback, frames, playing: false)
    #expect(view.shownContents != nil)
    let stopped = frames.stream.decodes
    update(view, playback, frames, playing: true)
    #expect(frames.stream.preparations == 2)
    await eventually("the gif decoded again after it came back") { frames.stream.decodes > stopped }
    withExtendedLifetime(window) {}
  }

  @Test
  func a_gif_without_a_window_decodes_nothing_and_resumes_when_it_joins_one() async throws {
    let frames = try decodedFrames()
    let view = FrameLayerView()
    let playback = AnimatedFramesView.Playback()

    update(view, playback, frames, playing: true)
    #expect(frames.stream.decodes == 0)
    #expect(frames.stream.preparations == 0)
    let window = attach(view)
    await eventually("the gif decoded after it joined a window") { frames.stream.decodes > 0 }
    #expect(frames.stream.preparations == 1)
    view.removeFromSuperview()
    let stopped = frames.stream.decodes
    window.addSubview(view)
    #expect(frames.stream.preparations == 2)
    await eventually("the gif decoded again after it rejoined the window") { frames.stream.decodes > stopped }
  }

  @Test
  func a_gif_that_left_its_window_ends_its_playback_task() async throws {
    var frames: AnimatedFrames? = try decodedFrames()
    let view = FrameLayerView()
    let playback = AnimatedFramesView.Playback()
    let window = attach(view)
    weak var stream = frames?.stream

    update(view, playback, try #require(frames), playing: true)
    await eventually("the stream decoded a frame") { stream?.decodes ?? 0 > 0 }
    view.removeFromSuperview()
    playback.plan = nil
    frames = nil
    await eventually("the cancelled playback task released the stream") { stream == nil }
    withExtendedLifetime(window) {}
  }

  // mutation: FrameLayerView.tick stops writing layer.contents, or show(_:) stops counting framesShown
  @Test
  func a_playing_gif_puts_changing_frames_on_its_layer_from_the_display_link() async throws {
    let frames = try decodedFrames()
    let view = FrameLayerView()
    let window = attach(view)
    let playback = AnimatedFramesView.Playback()

    update(view, playback, frames, playing: true)
    #expect(view.isPresenting)
    let start = view.framesShown
    var shown = Set<ObjectIdentifier>()
    await eventually("the display link put two different frames on the layer") {
      if let contents = view.shownContents { shown.insert(ObjectIdentifier(contents)) }
      return view.framesShown >= start + 2 && shown.count >= 2
    }
    #expect(view.shownContents != nil)
    withExtendedLifetime(window) {}
  }

  // mutation: stopPresenting() keeps the display link, or Playback.sync no longer stops the presenter when the view leaves its window
  @Test(arguments: PresentingStop.allCases)
  func a_gif_that_stops_presenting_writes_no_more_frames(_ stop: PresentingStop) async throws {
    let frames = try decodedFrames()
    let view = FrameLayerView()
    let window = attach(view)
    let playback = AnimatedFramesView.Playback()

    update(view, playback, frames, playing: true)
    let start = view.framesShown
    await eventually("the display link put a frame on the layer") { view.framesShown > start }
    switch stop {
    case .stopPresenting: view.stopPresenting()
    case .leftWindow: view.removeFromSuperview()
    }
    #expect(!view.isPresenting)
    let stopped = view.framesShown
    await displayFrames(30)
    #expect(view.framesShown == stopped)
    playback.reset()
    withExtendedLifetime(window) {}
  }

  // mutation: AnimatedFramesView.displayed ignores grayscale, or the plan drops the grayscale flag
  @Test(arguments: [false, true], [false, true])
  func a_grayscale_gif_shows_monochrome_frames_on_its_layer(grayscale: Bool, playing: Bool) async throws {
    let frames = try decodedFrames()
    let view = FrameLayerView()
    let window = attach(view)
    let playback = AnimatedFramesView.Playback()

    update(view, playback, frames, playing: playing, grayscale: grayscale)
    if playing {
      let start = view.framesShown
      await eventually("the display link put a frame on the layer") { view.framesShown > start }
    }
    let expected: CGColorSpaceModel? = grayscale ? .monochrome : frames.firstImage.colorSpace?.model
    #expect(view.shownContents?.colorSpace?.model == expected)
    playback.reset()
    withExtendedLifetime(window) {}
  }
}
#endif
