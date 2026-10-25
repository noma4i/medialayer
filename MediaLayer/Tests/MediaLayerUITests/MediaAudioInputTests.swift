#if canImport(UIKit)
import Foundation
import MediaLayer
import SwiftUI
import Testing
import UIKit
@testable import MediaLayerUI

private struct InputSource: MediaSource {
  let url: String
  let kind: Kind
  var headers: [String: String] { [:] }
}

private final class InputSlot: PlayerSlot, PlayerSlotFactory, @unchecked Sendable {
  private let lock = NSLock()
  private var events: [String] = []
  var all: [String] { lock.withLock { events } }
  var transport: String? { all.last { $0 == "play" || $0 == "pause" } }

  private func note(_ event: String) { lock.withLock { events.append(event) } }
  func create(slot: UInt32) -> PlayerSlot { self }
  func load(key: String, route: PlaybackRoute) {}
  func unload() {}
  func preroll() {}
  func play() { note("play") }
  func pause() { note("pause") }
  func mute() {}
  func unmute() {}
  func attach(viewToken: UInt64) {}
  func detach() {}
  func seek(positionMs: UInt64) {}
  func setRate(rate: Float) { note("rate") }
  func setReadahead(ms: UInt64) {}
  func sample() -> SlotSample { SlotSample(positionMs: 0, durationMs: nil, isLooping: false, viewAttached: false, frameRendered: false) }
}

@MainActor
private final class AudioInput: ObservableObject {
  @Published var url: String
  @Published var playing: Bool
  @Published var rate: Float = 1
  @Published var muted = false
  @Published var group = "ring"
  @Published var unrelated = 0
  var seen: [AudioPlayback] = []

  init(url: String, playing: Bool) {
    self.url = url
    self.playing = playing
  }

  var audio: AudioPlayback? { seen.last }
  var sameObject: Bool { seen.allSatisfy { $0 === seen.first } }
}

private struct InputCell: View {
  @ObservedObject var input: AudioInput
  let context: MediaContext

  var body: some View {
    MediaAudio(InputSource(url: input.url, kind: .audio), playing: input.playing, muted: input.muted, rate: input.rate, group: input.group) { playback in
      let _ = input.seen.append(playback)
      Color.clear
    }
    .environment(\.mediaContext, context)
    .frame(width: 80, height: 80)
  }
}

private func openInputCore(_ slot: InputSlot) throws -> MediaLayer {
  try MediaLayer.open(config: OpenConfig(
    dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
    availableBytes: UInt64.max, fetcher: UnusedFetcher(), settings: QuietSettings(), slots: slot,
    device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
    defaults: defaultLimits(), pool: defaultPoolSettings()
  ))
}

private func settles(_ condition: () -> Bool) async -> Bool {
  for _ in 0..<400 {
    if condition() { return true }
    try? await Task.sleep(for: .milliseconds(25))
  }
  return condition()
}

extension GlobalMedia {
  @MainActor
  @Suite(.serialized, .timeLimit(.minutes(2)))
  struct MediaAudioInputTests {
    private func mount(_ input: AudioInput, _ media: MediaLayer) async -> UIWindow {
      let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 200))
      window.rootViewController = UIHostingController(rootView: InputCell(input: input, context: MediaContext(places: media)))
      window.isHidden = false
      _ = await settles { input.audio != nil }
      return window
    }

    private func unmount(_ window: UIWindow, _ media: MediaLayer) {
      media.releaseAll()
      window.isHidden = true
      window.rootViewController = nil
    }

    @Test
    func aMountedAudioFollowsAChangingPlayingInputWithoutRemount() async throws {
      let slot = InputSlot()
      let media = try openInputCore(slot)
      let input = AudioInput(url: "file:///clip.mp4", playing: false)
      let window = await mount(input, media)
      defer { unmount(window, media) }
      let first = try #require(input.audio)
      #expect(slot.transport != "play")

      input.playing = true
      #expect(await settles { slot.transport == "play" })
      #expect(first.isPlaying)

      input.playing = false
      #expect(await settles { slot.transport == "pause" })
      #expect(!first.isPlaying)
      #expect(input.seen.count > 1)
      #expect(input.sameObject)
      #expect(input.audio === first)
    }

    @Test
    func aMountedAudioStartsAtAnInitialTruePlayingInput() async throws {
      let slot = InputSlot()
      let media = try openInputCore(slot)
      let input = AudioInput(url: "file:///clip.mp4", playing: true)
      let window = await mount(input, media)
      defer { unmount(window, media) }

      #expect(await settles { slot.transport == "play" })
      #expect(input.audio?.isPlaying == true)
    }

    @Test
    func aLocalToggleSurvivesRerendersThatLeavePlayingUnchanged() async throws {
      let slot = InputSlot()
      let media = try openInputCore(slot)
      let input = AudioInput(url: "file:///clip.mp4", playing: false)
      let window = await mount(input, media)
      defer { unmount(window, media) }
      let audio = try #require(input.audio)

      audio.toggle()
      #expect(await settles { slot.transport == "play" })
      input.unrelated += 1
      input.rate = 2
      input.muted = true
      input.group = "system"
      #expect(await settles { slot.all.last == "rate" })
      try await Task.sleep(for: .milliseconds(200))

      #expect(audio.isPlaying)
      #expect(slot.transport == "play")
      #expect(!slot.all.contains("pause"))
    }

    @Test
    func aChangedSourceStartsFromTheCurrentPlayingInput() async throws {
      let slot = InputSlot()
      let media = try openInputCore(slot)
      let input = AudioInput(url: "file:///first.mp4", playing: true)
      let window = await mount(input, media)
      defer { unmount(window, media) }
      let first = try #require(input.audio)
      #expect(await settles { slot.transport == "play" })

      let plays = slot.all.filter { $0 == "play" }.count

      input.url = "file:///second.mp4"
      input.playing = false
      #expect(await settles { input.audio !== first })
      try await Task.sleep(for: .milliseconds(200))
      #expect(input.audio?.isPlaying == false)
      #expect(slot.all.filter { $0 == "play" }.count == plays)

      input.url = "file:///third.mp4"
      input.playing = true
      #expect(await settles { slot.all.filter { $0 == "play" }.count > plays })
      #expect(input.audio?.isPlaying == true)
    }
  }
}
#endif
