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

private final class InputSlot: PlayerSlotFactory, @unchecked Sendable {
  private let lock = NSLock()
  private var events: [(slot: UInt32, name: String, key: String)] = []
  var all: [String] { lock.withLock { events.map { "\($0.slot):\($0.name):\($0.key)" } } }

  func playing(_ key: String) -> Bool {
    var loaded: [UInt32: String] = [:]
    var active: Set<UInt32> = []
    for event in lock.withLock({ events }) {
      switch event.name {
      case "load": loaded[event.slot] = event.key; active.remove(event.slot)
      case "play": active.insert(event.slot)
      case "pause", "unload": active.remove(event.slot)
      default: break
      }
    }
    return active.contains { loaded[$0] == key }
  }

  func count(_ name: String) -> Int { lock.withLock { events.filter { $0.name == name }.count } }

  func note(slot: UInt32, name: String, key: String) { lock.withLock { events.append((slot, name, key)) } }
  func create(slot: UInt32) async -> CreatedPlayer { CreatedPlayer(player: Recorder(slot: slot, log: self)) }

  private final class Recorder: PlayerSlot, @unchecked Sendable {
    let slot: UInt32
    let log: InputSlot
    private let lock = NSLock()
    private var key = ""
    init(slot: UInt32, log: InputSlot) {
      self.slot = slot
      self.log = log
    }

    private func note(_ name: String) { log.note(slot: slot, name: name, key: lock.withLock { key }) }
    func apply(command: SlotCall) async {
      switch command {
      case .load(let key, _, _): lock.withLock { self.key = key }; note("load")
      case .unload: note("unload")
      case .play: note("play")
      case .pause: note("pause")
      case .rate: note("rate")
      default: break
      }
    }
    func dispose() async {}
    func sample() -> SlotSample { SlotSample(positionMs: 0, durationMs: nil, isLooping: false, viewAttached: false, frameRendered: false) }
  }
}

private final class MountPlacesLog: MediaPlaces, @unchecked Sendable {
  private let inner: MediaLayer
  private let lock = NSLock()
  private var entries: [String] = []
  var calls: [String] { lock.withLock { entries } }

  init(_ inner: MediaLayer) { self.inner = inner }

  private func record(_ entry: String) { lock.withLock { entries.append(entry) } }

  func imageStats() -> ImageCounters { inner.imageStats() }
  func cacheKey(url: String, headers: [String: String]) -> String { inner.cacheKey(url: url, headers: headers) }
  func fetchImage(url: String, headers: [String: String]) async throws -> ImageFetchResult { try await inner.fetchImage(url: url, headers: headers) }
  func placeholder(source: MediaItem, ready: Bool) async -> Placeholder { await inner.placeholder(source: source, ready: ready) }
  func tonePixelsNeeded(source: MediaItem) async -> Bool { await inner.tonePixelsNeeded(source: source) }
  func playUrl(source: MediaItem) async throws -> PlaybackRoute { try await inner.playUrl(source: source) }
  func stateStream(key: String) -> AsyncStream<MediaState> { inner.stateStream(key: key) }
  func toneStream(source: MediaItem, band: Float) -> AsyncStream<ToneUpdate> { inner.toneStream(source: source, band: band) }
  func mountPlace(item: MediaItem) throws -> any PlaceHandle { record("mountPlace \(item.url)"); return try inner.mountPlace(item: item) }
  func mountVideoWindow() throws -> any WindowHandle { try inner.mountVideoWindow() }
  func mountImageWindow(name: String, rowsPerPage: Int, reversed: Bool) throws -> any WindowHandle {
    try inner.mountImageWindow(name: name, rowsPerPage: rowsPerPage, reversed: reversed)
  }
  func seek(key: String, fraction: Double) throws { try inner.seek(key: key, fraction: fraction) }
  func setMutedAll(muted: Bool) throws { try inner.setMutedAll(muted: muted) }
  func setTonePreparation(preparation: TonePreparation) throws { try inner.setTonePreparation(preparation: preparation) }
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
  var toggleOnMount = false
  var order: [String] = []

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
      Color.clear.onAppear {
        if input.toggleOnMount {
          input.toggleOnMount = false
          input.order.append("child onAppear toggle")
          playback.toggle()
        }
      }
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
    private func mount(_ input: AudioInput, _ media: MediaLayer, places: MediaPlaces? = nil) async -> UIWindow {
      let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 200, height: 200))
      window.rootViewController = UIHostingController(rootView: InputCell(input: input, context: MediaContext(places: places ?? media)))
      window.isHidden = false
      _ = await settles { input.audio != nil }
      return window
    }

    private func unmount(_ window: UIWindow, _ media: MediaLayer) {
      media.releaseOwned()
      window.isHidden = true
      window.rootViewController = nil
    }

    private func barrier(_ input: AudioInput, _ media: MediaLayer, _ slot: InputSlot, key: String) async {
      let renders = input.seen.count
      input.unrelated += 1
      _ = await settles { input.seen.count > renders }
      let before = slot.count("rate")
      media.setRate(key: key, rate: Float(3 + before))
      _ = await settles { slot.count("rate") > before }
    }

    @Test
    func aMountedAudioFollowsAChangingPlayingInputWithoutRemount() async throws {
      let slot = InputSlot()
      let media = try openInputCore(slot)
      let input = AudioInput(url: "file:///clip.mp4", playing: false)
      let window = await mount(input, media)
      defer { unmount(window, media) }
      let first = try #require(input.audio)
      #expect(slot.count("play") == 0)

      input.playing = true
      #expect(await settles { slot.playing("file:///clip.mp4") })
      #expect(first.isPlaying)

      input.playing = false
      #expect(await settles { !slot.playing("file:///clip.mp4") })
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

      #expect(input.audio?.isPlaying == true)
      #expect(await settles { slot.playing("file:///clip.mp4") })
    }

    @Test
    func aLocalToggleAtMountSurvivesAnInitialFalsePlayingInput() async throws {
      let slot = InputSlot()
      let media = try openInputCore(slot)
      let places = MountPlacesLog(media)
      let input = AudioInput(url: "file:///clip.mp4", playing: false)
      input.toggleOnMount = true
      let window = await mount(input, media, places: places)
      defer { unmount(window, media) }

      let played = await settles { slot.playing("file:///clip.mp4") }
      #expect(input.order == ["child onAppear toggle"])
      #expect(places.calls == ["mountPlace file:///clip.mp4"])
      #expect(played)
      await barrier(input, media, slot, key: "file:///clip.mp4")
      #expect(input.audio?.isPlaying == true)
      #expect(slot.playing("file:///clip.mp4"))
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
      #expect(await settles { slot.playing("file:///clip.mp4") })
      input.rate = 2
      input.muted = true
      input.group = "system"
      await barrier(input, media, slot, key: "file:///clip.mp4")

      #expect(audio.isPlaying)
      #expect(slot.playing("file:///clip.mp4"))
      #expect(slot.count("pause") == 0)
      #expect(input.sameObject)
    }

    @Test
    func aChangedSourceReleasesTheOldOneAndStartsFromTheCurrentPlayingInput() async throws {
      let slot = InputSlot()
      let media = try openInputCore(slot)
      let input = AudioInput(url: "file:///first.mp4", playing: true)
      let window = await mount(input, media)
      defer { unmount(window, media) }
      let first = try #require(input.audio)
      #expect(await settles { slot.playing("file:///first.mp4") })

      input.url = "file:///second.mp4"
      input.playing = false
      #expect(await settles { input.audio !== first })
      #expect(input.audio?.isPlaying == false)

      input.url = "file:///third.mp4"
      input.playing = true
      #expect(await settles { slot.playing("file:///third.mp4") })
      #expect(input.audio?.isPlaying == true)
      #expect(!slot.all.contains { $0.contains("play:file:///second.mp4") })
      #expect(!slot.playing("file:///first.mp4"))
    }
  }
}
#endif
