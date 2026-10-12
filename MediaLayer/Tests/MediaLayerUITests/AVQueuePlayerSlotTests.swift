import AVFoundation
import Foundation
import MediaLayer
import QuartzCore
import Testing
@testable import MediaLayerUI

@Suite(.serialized, .timeLimit(.minutes(1)))
struct AVQueuePlayerSlotTests {
  private static var silentClip: String {
    Bundle.module.url(forResource: "silent", withExtension: "mp4", subdirectory: "Fixtures")!.absoluteString
  }

  private func makeSlot(_ log: PlayerEventLog, slots: LoadedSlots = LoadedSlots()) -> AVQueuePlayerSlot {
    let factory = AVQueuePlayerSlotFactory(startMuted: true, slots: slots)
    factory.events.bind(log)
    return factory.create(slot: 3) as! AVQueuePlayerSlot
  }

  private func route(_ url: String, headers: [String: String] = [:], hls: Bool = false, looping: Bool = false) -> PlaybackRoute {
    PlaybackRoute(url: url, headers: headers, generation: nil, hls: hls, looping: looping)
  }

  private func drainMain() async {
    await MainActor.run {}
    await MainActor.run {}
  }

  @Test
  func aSlotLoadedForANewKeyLeavesTheHostOfTheOldKey() async {
    let log = PlayerEventLog()
    let slot = makeSlot(log)
    let host = CALayer()
    slot.load(key: "a", route: route(Self.silentClip))
    await log.wait(for: "ready")
    slot.attach(to: host)
    await drainMain()
    #expect(slot.layer.superlayer === host)

    slot.load(key: "b", route: route(Self.silentClip))
    await drainMain()

    #expect(slot.layer.superlayer == nil)
  }

  @Test
  func theReadaheadOfTheCoreReachesEveryItemOfTheSlotAndTheLoopCopy() async {
    let log = PlayerEventLog()
    let slot = makeSlot(log)
    slot.load(key: "a", route: route(Self.silentClip, looping: true))
    await log.wait(for: "ready")

    slot.setReadahead(ms: 12_000)
    await drainMain()

    let durations = await MainActor.run { slot.player.items().map(\.preferredForwardBufferDuration) }
    #expect(durations.count == 2 && durations.allSatisfy { $0 == 12 })
  }

  @Test
  func reloadingTheSameKeyKeepsTheHost() async {
    let log = PlayerEventLog()
    let slot = makeSlot(log)
    let host = CALayer()
    slot.load(key: "a", route: route(Self.silentClip))
    slot.attach(to: host)
    await drainMain()

    slot.unload()
    slot.load(key: "a", route: route(Self.silentClip))
    await drainMain()

    #expect(slot.layer.superlayer === host)
  }

  @Test
  func localClipPlaysMutedFromReadyToEnded() async throws {
    let log = PlayerEventLog()
    let slot = makeSlot(log)

    slot.load(key: "clip", route: route(Self.silentClip))
    await log.wait(for: "ready")
    slot.play()
    await log.wait(for: "ended")

    #expect(slot.player.isMuted)
    #expect(log.count("failed 0") == 0)
    #expect(slot.sample().durationMs != nil)
    #expect(slot.player.currentItem != nil)
  }

  @Test
  func theRateIsTheDefaultRateOfThePlayerAndGoesBackToNormalWithTheItem() async throws {
    let log = PlayerEventLog()
    let slot = makeSlot(log)
    slot.load(key: "clip", route: route(Self.silentClip))

    slot.setRate(rate: 1.5)
    await drainMain()
    #expect(slot.player.defaultRate == 1.5)
    slot.unload()
    await drainMain()

    #expect(slot.player.defaultRate == 1)
  }

  @Test
  func unloadFreesTheItemAndTheLoadedKey() async throws {
    let log = PlayerEventLog()
    let slots = LoadedSlots()
    let slot = makeSlot(log, slots: slots)
    slot.load(key: "clip", route: route(Self.silentClip))
    await log.wait(for: "ready")
    weak var item = slot.loadedItem
    #expect(slots.slot(for: "clip") === slot)

    slot.unload()
    await drainMain()

    #expect(item == nil)
    #expect(slot.player.items().isEmpty)
    #expect(slots.slot(for: "clip") == nil)
    #expect(slot.sample().frameRendered == false)
  }

  @Test
  func droppingALoadedSlotLeavesNoRetainCycle() async throws {
    let log = PlayerEventLog()
    weak var dropped: AVQueuePlayerSlot?
    weak var item: AVPlayerItem?
    do {
      let slot = makeSlot(log)
      dropped = slot
      slot.load(key: "clip", route: route(Self.silentClip))
      await log.wait(for: "ready")
      item = slot.loadedItem
    }
    await drainMain()
    await drainMain()

    #expect(dropped == nil)
    #expect(item == nil)
  }

  @Test
  func muteAndUnmuteTouchOnlyThePlayerFlag() async throws {
    let slot = makeSlot(PlayerEventLog())

    slot.unmute()
    await drainMain()
    #expect(!slot.player.isMuted)
    slot.mute()
    await drainMain()

    #expect(slot.player.isMuted)
    #expect(slot.player.volume == 1)
  }

  @Test
  func routeHeadersReachTheOrigin() async throws {
    let server = try LocalHTTPServer()
    try await server.start()
    defer { server.stop() }
    let log = PlayerEventLog()
    let slot = makeSlot(log)
    var requests = server.requests.makeAsyncIterator()

    slot.load(key: "remote", route: route(server.url("/headers"), headers: ["X-Test": "1"]))
    _ = await requests.next()

    #expect(server.headers(of: "/headers")["x-test"] == "1")
  }

  @Test
  func unplayableSourceReportsFailed() async throws {
    let log = PlayerEventLog()
    let slot = makeSlot(log)

    slot.load(key: "missing", route: route("file:///nonexistent/clip.mp4"))
    await log.wait(for: "failed 0")

    #expect(log.count("ready") == 0)
  }

  @Test
  func coreAttachPutsTheLayerInTheRegisteredViewAndDetachTakesItOut() async throws {
    let hosts = HostLayers()
    let factory = AVQueuePlayerSlotFactory(startMuted: true, slots: LoadedSlots(), hosts: hosts)
    let slot = factory.create(slot: 3) as! AVQueuePlayerSlot
    let host = CALayer()
    let token = hosts.register(host)

    slot.attach(viewToken: token)
    await drainMain()
    #expect(host.sublayers == [slot.layer])
    #expect(slot.sample().viewAttached)
    slot.detach()
    await drainMain()

    #expect(host.sublayers == nil)
    #expect(!slot.sample().viewAttached)
  }

  @Test(arguments: [(true, 0), (false, 1)])
  func endOfClipIsReportedOnlyWhenNotLooping(looping: Bool, endedEvents: Int) async throws {
    let log = PlayerEventLog()
    let slot = makeSlot(log)
    slot.load(key: "clip", route: route(Self.silentClip, looping: looping))
    await log.wait(for: "ready")

    NotificationCenter.default.post(name: .AVPlayerItemDidPlayToEndTime, object: slot.loadedItem)
    await drainMain()
    await drainMain()

    #expect(log.count("ended") == endedEvents)
    #expect(slot.sample().isLooping == looping)
    #expect(slot.player.actionAtItemEnd == (looping ? .advance : .pause))
  }

  @Test(arguments: [(true, 2), (false, 1)])
  func aLoopingClipKeepsTheNextRoundQueued(looping: Bool, queued: Int) async throws {
    let log = PlayerEventLog()
    let slot = makeSlot(log)
    slot.load(key: "clip", route: route(Self.silentClip, looping: looping))
    await log.wait(for: "ready")
    await drainMain()
    let first = slot.loadedItem

    #expect(slot.player.items().count == queued)
    guard looping else { return }

    await MainActor.run { slot.player.advanceToNextItem() }
    await drainMain()
    await drainMain()

    #expect(slot.loadedItem !== first)
    #expect(slot.player.items().count == queued)
  }

  @Test
  func seekMovesThePlayhead() async throws {
    let log = PlayerEventLog()
    let slot = makeSlot(log)
    slot.load(key: "clip", route: route(Self.silentClip))
    await log.wait(for: "ready")

    slot.seek(positionMs: 300)

    await eventually("the playhead reached 0.29 s") { await MainActor.run { slot.player.currentTime().seconds >= 0.29 } }
  }

  @Test
  func inv_idle_05_a_slot_sample_reads_the_live_playhead_without_a_periodic_observer() async {
    let log = PlayerEventLog()
    let slot = makeSlot(log)
    slot.load(key: "clip", route: route(Self.silentClip))
    await log.wait(for: "ready")

    slot.seek(positionMs: 300)

    await eventually("sample() reports the sought position") { slot.sample().positionMs >= 290 }
  }

  @Test(arguments: [
    (["X-Test": "1"], true, ["AVURLAssetHTTPHeaderFieldsKey", "AVURLAssetOverrideMIMETypeKey"]),
    (["X-Test": "1"], false, ["AVURLAssetHTTPHeaderFieldsKey"]),
    ([:], true, ["AVURLAssetOverrideMIMETypeKey"]),
    ([:], false, [])
  ])
  func assetOptionsFollowTheRoute(headers: [String: String], hls: Bool, keys: [String]) {
    let options = AVQueuePlayerSlot.assetOptions(for: route("https://o.test/a", headers: headers, hls: hls))

    #expect(Set(options?.keys.map { $0 } ?? []) == Set(keys))
  }

  @MainActor
  @Test
  func audioSeekSendsAFractionOfTheDurationToTheCore() async throws {
    let log = PlayerEventLog()
    let slots = LoadedSlots()
    let slot = makeSlot(log, slots: slots)
    let places = RecordingPlaces()
    slot.load(key: "note", route: route(Self.silentClip))
    await log.wait(for: "ready")
    let duration = try #require(slot.sample().durationMs)
    let audio = AudioPlayback(key: "note", playing: false, places: places, slots: slots)

    audio.seek(to: 0.5)

    #expect(places.recorded == ["seek \(duration / 2)"])
  }
}
