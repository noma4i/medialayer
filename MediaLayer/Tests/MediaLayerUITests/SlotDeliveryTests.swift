import AVFoundation
import Foundation
import MediaLayer
import Testing
@testable import MediaLayerUI

@Suite(.serialized, .timeLimit(.minutes(1)))
struct SlotDeliveryTests {
  private static var silentClip: String {
    Bundle.module.url(forResource: "silent", withExtension: "mp4", subdirectory: "Fixtures")!.absoluteString
  }

  private func route(looping: Bool = false) -> PlaybackRoute {
    PlaybackRoute(url: Self.silentClip, headers: [:], generation: nil, hls: false, looping: looping)
  }

  private func makeSlot(_ log: PlayerEventLog, hosts: HostLayers = HostLayers()) async -> AVQueuePlayerSlot {
    let factory = AVQueuePlayerSlotFactory(startMuted: true, slots: LoadedSlots(), hosts: hosts)
    factory.events.bind(log)
    return await factory.makeSlot(5)
  }

  @Test
  func inv_api_42_applyReturnsAfterTheNativeSideChangedNotAfterThePost() async {
    let hosts = HostLayers()
    let slot = await makeSlot(PlayerEventLog(), hosts: hosts)
    let host = CALayer()
    let token = hosts.register(host)

    await slot.apply(command: .attach(viewToken: token))
    let attached = slot.sample().viewAttached
    await slot.apply(command: .detach)
    let detached = !slot.sample().viewAttached

    #expect(attached, "attach returned before the layer joined the host")
    #expect(detached, "detach returned before the layer left the host")
    withExtendedLifetime(host) {}
  }

  @Test
  func inv_api_42_aCommandOfACancelledTaskOrAClosedSlotIsNotApplied() async {
    let slot = await makeSlot(PlayerEventLog())
    let load = SlotCall.load(key: "a", route: route(), generation: 1)

    let task = await MainActor.run {
      let task = Task { await slot.apply(command: load) }
      task.cancel()
      return task
    }
    await task.value
    let afterCancel = await MainActor.run { slot.loadedItem != nil }
    slot.close()
    await slot.apply(command: load)
    let afterClose = await MainActor.run { slot.loadedItem != nil }

    #expect(!afterCancel, "a cancelled apply still loaded")
    #expect(!afterClose, "a closed slot was revived by a late command")
  }

  @Test
  func inv_api_43_theEventsOfAnItemCarryTheGenerationOfItsLoadAndAnOldItemOrGenerationSaysNothing() async throws {
    let log = PlayerEventLog()
    let slot = await makeSlot(log)
    await slot.apply(command: .load(key: "a", route: route(), generation: 7))
    await log.wait(for: "ready")
    let old = try #require(await MainActor.run { slot.loadedItem })
    await slot.apply(command: .load(key: "a", route: route(), generation: 8))
    await eventually("the current load is ready") { log.generations("ready") == [7, 8] }
    let current = try #require(await MainActor.run { slot.loadedItem })

    await MainActor.run {
      slot.statusChanged(old, generation: 7)
      slot.itemEnded(old, generation: 7)
      slot.itemEnded(current, generation: 7)
    }
    #expect(log.count("ended") == 0, "a stale end reached the core: \(log.generations("ended"))")
    #expect(log.generations("ready") == [7, 8], "a stale status reached the core: \(log.generations("ready"))")
    await MainActor.run { slot.itemEnded(current, generation: 8) }

    #expect(log.generations("ended") == [8])
  }
}
