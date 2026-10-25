import Foundation
import MediaLayer
import Testing
@testable import MediaLayerUI

@Suite(.serialized, .timeLimit(.minutes(1)))
struct PlayerSnapshotTests {
  private static var silentClip: String {
    Bundle.module.url(forResource: "silent", withExtension: "mp4", subdirectory: "Fixtures")!.absoluteString
  }

  @Test
  func inv_native_34_playerSnapshotFollowsPlayPauseMuteSeekAndRate() async {
    let log = PlayerEventLog()
    let slots = LoadedSlots()
    let factory = AVQueuePlayerSlotFactory(startMuted: true, slots: slots)
    factory.events.bind(log)
    let slot = await factory.makeSlot(3)
    #expect(slots.snapshot(for: "k") == nil)

    await slot.load(key: "k", route: PlaybackRoute(url: Self.silentClip, headers: [:], generation: nil, hls: false, looping: true))
    await log.wait(for: "ready")
    #expect(slots.snapshot(for: "k") == PlayerSnapshot(key: "k", isPlaying: false, isMuted: true, positionMs: 0, rate: 1))

    await slot.play()
    #expect(slots.snapshot(for: "k")?.isPlaying == true)
    await slot.pause()
    #expect(slots.snapshot(for: "k")?.isPlaying == false)
    await slot.unmute()
    #expect(slots.snapshot(for: "k")?.isMuted == false)
    await slot.mute()
    #expect(slots.snapshot(for: "k")?.isMuted == true)
    await slot.setRate(rate: 2)
    #expect(slots.snapshot(for: "k")?.rate == 2)
    await slot.seek(positionMs: 500)
    #expect((slots.snapshot(for: "k")?.positionMs ?? 0) >= 400)

    await slot.dispose()
    #expect(slots.snapshot(for: "k") == nil)
  }
}
