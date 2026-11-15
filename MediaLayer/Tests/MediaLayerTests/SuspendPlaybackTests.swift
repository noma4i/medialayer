import Foundation
import Testing
@testable import MediaLayer
import MediaLayerTesting

private final class TransportSlot: PlayerSlot, PlayerSlotFactory, @unchecked Sendable {
  let log = AsyncStream<String>.makeStream()

  func create(slot: UInt32) async -> CreatedPlayer { CreatedPlayer(player: self) }
  func apply(command: SlotCall) async {
    switch command {
    case .play: log.continuation.yield("play")
    case .pause: log.continuation.yield("pause")
    case .rate: log.continuation.yield("rate")
    default: break
    }
  }
  func dispose() async {}
  func sample() -> SlotSample {
    SlotSample(positionMs: 0, durationMs: nil, isLooping: false, viewAttached: false, frameRendered: false)
  }
}

private final class QuietSettings: SettingsStore, @unchecked Sendable {
  func read(key: String) -> String? { nil }
  func write(key: String, value: String) {}
  func remove(key: String) {}
}

private let clip = "file:///clip.mp4"

private func openPlaying(groups: [String?] = [nil]) async throws -> (MediaLayer, AsyncStream<String>.Iterator) {
  let slot = TransportSlot()
  let media = try MediaLayer.open(config: OpenConfig(
    dir: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString).path,
    fetcher: TestFetcher(), settings: QuietSettings(), slots: slot,
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
  func aSuspensionReleasedByHandOrDroppedResumesThePlayback() async throws {
    var (media, events) = try await openPlaying()
    let released = try media.suspendPlayback()
    #expect(await events.next() == "pause")
    released.release()
    #expect(await events.next() == "play")

    var dropped: PlaybackSuspension? = try media.suspendPlayback()
    #expect(await events.next() == "pause")
    dropped = nil
    #expect(await events.next() == "play")
    withExtendedLifetime(media) {}
  }

  @Test
  func theAllowedGroupsOfASuspensionReachTheCore() async throws {
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
}
