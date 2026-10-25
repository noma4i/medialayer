#if canImport(UIKit)
import Foundation
import MediaLayer
import SwiftUI
import Testing
import UIKit
@testable import MediaLayerUI

private let clip = "file:///clip.mp4"

private struct ClipSource: MediaSource {
  let url: String
  let kind: Kind
  var headers: [String: String] { [:] }
}

private final class TransportSlot: PlayerSlot, PlayerSlotFactory, @unchecked Sendable {
  private let lock = NSLock()
  private var events: [String] = []
  var all: [String] { lock.withLock { events } }
  var transport: String? { all.last { $0 == "play" || $0 == "pause" } }

  private func note(_ event: String) { lock.withLock { events.append(event) } }
  func create(slot: UInt32) async -> CreatedPlayer { CreatedPlayer(player: self) }
  func apply(command: SlotCall) async {
    switch command {
    case .play: note("play")
    case .pause: note("pause")
    case .rate: note("rate")
    default: break
    }
  }
  func sample() -> SlotSample { SlotSample(positionMs: 0, durationMs: nil, isLooping: false, viewAttached: false, frameRendered: false) }
}

@MainActor
private final class GroupInput: ObservableObject {
  @Published var group = "ring"
  var audio: AudioPlayback?
}

private struct Cell: View {
  @ObservedObject var input: GroupInput
  let context: MediaContext
  let audio: Bool
  var audioPlaying = true

  var body: some View {
    Group {
      if audio {
        MediaAudio(ClipSource(url: clip, kind: .audio), playing: audioPlaying, group: input.group) { playback in
          Color.clear.onAppear { input.audio = playback }
        }
      } else {
        MediaVideo(ClipSource(url: clip, kind: .video), group: input.group)
      }
    }
    .environment(\.mediaContext, context)
    .frame(width: 80, height: 80)
  }
}

private func openCore(_ slot: TransportSlot) throws -> MediaLayer {
  try MediaLayer.open(config: OpenConfig(
    dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
    availableBytes: UInt64.max, fetcher: UnusedFetcher(), settings: QuietSettings(), slots: slot,
    device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
    defaults: defaultLimits(), pool: defaultPoolSettings()
  ))
}

private func until(_ condition: () -> Bool) async -> Bool {
  for _ in 0..<400 {
    if condition() { return true }
    try? await Task.sleep(for: .milliseconds(25))
  }
  return condition()
}

extension GlobalMedia {
  @MainActor
  @Suite(.serialized, .timeLimit(.minutes(2)))
  struct MountedGroupTests {
    private func followsItsGroup(audio: Bool) async throws -> Bool {
      let slot = TransportSlot()
      let media = try openCore(slot)
      let input = GroupInput()
      let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 200))
      let host = UIHostingController(rootView: Cell(input: input, context: MediaContext(places: media), audio: audio))
      window.rootViewController = host
      window.isHidden = false
      defer {
        media.releaseAll()
        window.isHidden = true
        window.rootViewController = nil
      }
      if audio {
        #expect(await until { input.audio != nil })
      }
      #expect(await until { slot.transport == "play" })
      let hold = try media.suspendPlayback(allowGroups: ["system"])
      #expect(await until { slot.transport == "pause" })

      input.group = "system"
      #expect(await until { slot.transport == "play" })
      input.group = "ring"
      let paused = await until { slot.transport == "pause" }
      withExtendedLifetime(hold) {}
      return paused
    }

    private func mountedToggledAudio(_ slot: TransportSlot, _ media: MediaLayer, _ input: GroupInput) async -> (UIWindow, AudioPlayback?) {
      let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 200))
      let host = UIHostingController(rootView: Cell(input: input, context: MediaContext(places: media), audio: true, audioPlaying: false))
      window.rootViewController = host
      window.isHidden = false
      _ = await until { input.audio != nil }
      input.audio?.toggle()
      _ = await until { slot.transport == "play" }
      return (window, input.audio)
    }

    @Test
    func inv_session_16_aToggledAudioKeepsPlayingWhenItsGroupChangesUnderASuspension() async throws {
      let slot = TransportSlot()
      let media = try openCore(slot)
      let input = GroupInput()
      let (window, audio) = await mountedToggledAudio(slot, media, input)
      defer {
        media.releaseAll()
        window.isHidden = true
        window.rootViewController = nil
      }
      #expect(audio?.isPlaying == true)
      let hold = try media.suspendPlayback(allowGroups: ["system"])
      #expect(await until { slot.transport == "pause" })

      input.group = "system"
      #expect(await until { slot.transport == "play" })
      withExtendedLifetime(hold) {}
    }

    @Test
    func inv_session_16_aToggledAudioStaysPlayingWhenAnotherViewOfItsKeyComesAndGoes() async throws {
      let slot = TransportSlot()
      let media = try openCore(slot)
      let input = GroupInput()
      let (window, _) = await mountedToggledAudio(slot, media, input)
      defer {
        media.releaseAll()
        window.isHidden = true
        window.rootViewController = nil
      }
      let claims = KeyClaims.of(media)
      let other = UUID()

      claims.setPlaying(owner: other, key: clip, playing: true, group: "chat")
      claims.release(owner: other, key: clip)
      media.setRate(key: clip, rate: 2)
      #expect(await until { slot.all.last == "rate" })
      #expect(slot.transport == "play")
    }

    @Test
    func inv_session_15_aMountedVideoFollowsAChangeOfItsGroupOnly() async throws {
      #expect(try await followsItsGroup(audio: false))
    }

    @Test
    func inv_session_15_aMountedAudioFollowsAChangeOfItsGroupOnly() async throws {
      #expect(try await followsItsGroup(audio: true))
    }

    @Test
    func inv_session_14_aKeyWithSeveralPlayingOwnersIsAllowedByAnyOfTheirGroups() async throws {
      let slot = TransportSlot()
      let media = try openCore(slot)
      let claims = KeyClaims(places: media)
      let item = MediaItem(kind: .video, url: clip, headers: [:], cover: nil, blurHash: nil, strategy: .window)
      let ring = UUID(), system = UUID()
      defer { media.releaseAll() }
      claims.visible(owner: ring, source: item, visible: true)
      claims.setPlaying(owner: ring, key: clip, playing: true, group: "ring")
      claims.setPlaying(owner: system, key: clip, playing: true, group: "system")
      #expect(await until { slot.transport == "play" })

      let allowing = try media.suspendPlayback(allowGroups: ["system"])
      media.setRate(key: clip, rate: 2)
      #expect(await until { slot.all.last == "rate" })
      #expect(slot.transport == "play")

      let blocking = try media.suspendPlayback(allowGroups: ["ring"])
      #expect(await until { slot.transport == "pause" })
      blocking.release()
      #expect(await until { slot.transport == "play" })
      withExtendedLifetime(allowing) {}
    }
  }
}
#endif
