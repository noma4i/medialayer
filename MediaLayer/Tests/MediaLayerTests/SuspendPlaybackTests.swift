import Foundation
import Testing
@testable import MediaLayer

private final class TransportSlot: PlayerSlot, PlayerSlotFactory, @unchecked Sendable {
  let log = AsyncStream<String>.makeStream()

  func create(slot: UInt32) -> PlayerSlot { self }
  func load(key: String, route: PlaybackRoute) {}
  func unload() {}
  func preroll() {}
  func play() { log.continuation.yield("play") }
  func pause() { log.continuation.yield("pause") }
  func mute() {}
  func unmute() {}
  func attach(viewToken: UInt64) {}
  func detach() {}
  func seek(positionMs: UInt64) {}
  func setRate(rate: Float) { log.continuation.yield("rate") }
  func setReadahead(ms: UInt64) {}
  func sample() -> SlotSample {
    SlotSample(positionMs: 0, durationMs: nil, isLooping: false, viewAttached: false, frameRendered: false)
  }
}

private final class QuietSettings: SettingsStore, @unchecked Sendable {
  func read(key: String) -> String? { nil }
  func write(key: String, value: String) {}
  func remove(key: String) {}
}

private final class OfflineFetcher: Fetcher, @unchecked Sendable {
  func fetch(request: FetchRequest) async throws -> FetchResponse { throw MediaError.Fetch(detail: "offline") }
}

private let clip = "file:///clip.mp4"

private func openPlaying(groups: [String?] = [nil]) async throws -> (MediaLayer, AsyncStream<String>.Iterator) {
  let slot = TransportSlot()
  let media = try MediaLayer.open(config: OpenConfig(
    dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
    availableBytes: UInt64.max, fetcher: OfflineFetcher(), settings: QuietSettings(), slots: slot,
    device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
    defaults: defaultLimits(), pool: defaultPoolSettings()
  ))
  media.replaceWindow(active: 0, sources: [MediaItem(kind: .video, url: clip, headers: [:], cover: nil, blurHash: nil, strategy: .window)])
  media.setPlaying(key: clip, playing: true, groups: groups)
  var events = slot.log.stream.makeAsyncIterator()
  #expect(await events.next() == "play")
  return (media, events)
}

@Suite(.timeLimit(.minutes(1)))
struct SuspendPlaybackTests {
  @Test
  func inv_session_12_the_last_release_resumes_and_the_first_one_does_not() async throws {
    var (media, events) = try await openPlaying()
    let first = try media.suspendPlayback()
    var second: PlaybackSuspension? = try media.suspendPlayback()
    #expect(await events.next() == "pause")

    first.release()
    first.release()
    media.setRate(key: clip, rate: 2)
    #expect(await events.next() == "rate")

    second = nil
    #expect(await events.next() == "play")
    withExtendedLifetime(media) {}
  }

  @Test
  func inv_session_12_a_group_every_suspension_allows_keeps_playing() async throws {
    var (media, events) = try await openPlaying(groups: ["chat"])
    let allowing = try media.suspendPlayback(allowGroups: ["chat"])
    media.setRate(key: clip, rate: 2)
    #expect(await events.next() == "rate")

    let blocking = try media.suspendPlayback(allowGroups: ["other"])
    #expect(await events.next() == "pause")
    blocking.release()
    #expect(await events.next() == "play")
    withExtendedLifetime((media, allowing)) {}
  }

  @Test
  func inv_session_17_an_ungrouped_owner_is_in_the_default_group_under_a_suspension() async throws {
    var (media, events) = try await openPlaying(groups: [nil, "system"])
    let allowing = try media.suspendPlayback(allowGroups: ["ring"])
    media.setRate(key: clip, rate: 2)
    #expect(await events.next() == "rate")

    let blocking = try media.suspendPlayback(allowGroups: ["system"])
    #expect(await events.next() == "pause")
    blocking.release()
    #expect(await events.next() == "play")
    withExtendedLifetime((media, allowing)) {}
  }

  @Test
  func inv_session_17_the_default_group_leaves_the_set_when_the_ungrouped_owner_leaves() async throws {
    var (media, events) = try await openPlaying(groups: [nil, "system"])
    let suspension = try media.suspendPlayback(allowGroups: ["ring"])
    media.setRate(key: clip, rate: 2)
    #expect(await events.next() == "rate")

    media.setPlaying(key: clip, playing: true, groups: ["system"])
    #expect(await events.next() == "pause")
    withExtendedLifetime((media, suspension)) {}
  }

  @Test
  func inv_session_13_a_blank_allow_group_throws() throws {
    let media = try MediaLayer.open(config: OpenConfig(
      dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
      availableBytes: UInt64.max, fetcher: OfflineFetcher(), settings: QuietSettings(), slots: nil,
      device: DeviceMemory(totalMb: 4096, heapClassMb: nil, lowRam: false),
      defaults: defaultLimits(), pool: defaultPoolSettings()
    ))
    #expect(throws: MediaError.self) { try media.suspendPlayback(allowGroups: [" "]) }
  }
}
