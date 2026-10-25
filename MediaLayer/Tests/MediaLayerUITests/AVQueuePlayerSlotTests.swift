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

  private func makeSlot(_ log: PlayerEventLog, slots: LoadedSlots = LoadedSlots()) async -> AVQueuePlayerSlot {
    let factory = AVQueuePlayerSlotFactory(startMuted: true, slots: slots)
    factory.events.bind(log)
    return await factory.makeSlot(3)
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
    let slot = await makeSlot(log)
    let host = CALayer()
    await slot.load(key: "a", route: route(Self.silentClip))
    await log.wait(for: "ready")
    await MainActor.run { slot.attach(to: host) }
    await drainMain()
    #expect(slot.layer.superlayer === host)

    await slot.load(key: "b", route: route(Self.silentClip))
    await drainMain()

    #expect(slot.layer.superlayer == nil)
  }

  @Test
  func theReadaheadOfTheCoreReachesEveryItemOfTheSlotAndTheLoopCopy() async {
    let log = PlayerEventLog()
    let slot = await makeSlot(log)
    await slot.load(key: "a", route: route(Self.silentClip, looping: true))
    await log.wait(for: "ready")

    await slot.setReadahead(ms: 12_000)
    await drainMain()

    let durations = await MainActor.run { slot.player.items().map(\.preferredForwardBufferDuration) }
    #expect(durations.count == 2 && durations.allSatisfy { $0 == 12 })
  }

  @Test
  func inv_ios_02_aReplacingLoadReleasesTheOldKeyAndStopsPlayback() async {
    let log = PlayerEventLog()
    let slots = LoadedSlots()
    let slot = await makeSlot(log, slots: slots)

    await slot.load(key: "old", route: route(Self.silentClip), generation: 1)
    await slot.seek(positionMs: 10)
    await slot.play()
    await slot.unload()
    await slot.load(key: "new", route: route(Self.silentClip), generation: 2)
    await slot.seek(positionMs: 20)
    await drainMain()

    #expect(slots.slot(for: "old") == nil)
    #expect(slots.slot(for: "new") === slot)
    let rate = await MainActor.run { slot.player.rate }
    #expect(rate == 0)
  }

  @Test
  func reloadingTheSameKeyKeepsTheHost() async {
    let log = PlayerEventLog()
    let slot = await makeSlot(log)
    let host = CALayer()
    await slot.load(key: "a", route: route(Self.silentClip))
    await MainActor.run { slot.attach(to: host) }
    await drainMain()

    await slot.unload()
    await slot.load(key: "a", route: route(Self.silentClip))
    await drainMain()

    #expect(slot.layer.superlayer === host)
  }

  @Test
  func localClipPlaysMutedFromReadyToEnded() async throws {
    let log = PlayerEventLog()
    let slot = await makeSlot(log)

    await slot.load(key: "clip", route: route(Self.silentClip))
    await log.wait(for: "ready")
    await slot.play()
    await log.wait(for: "ended")

    #expect(slot.player.isMuted)
    #expect(log.count("failed 0") == 0)
    #expect(slot.sample().durationMs != nil)
    #expect(slot.player.currentItem != nil)
  }

  @Test
  func theRateIsTheDefaultRateOfThePlayerAndGoesBackToNormalWithTheItem() async throws {
    let log = PlayerEventLog()
    let slot = await makeSlot(log)
    await slot.load(key: "clip", route: route(Self.silentClip))

    await slot.setRate(rate: 1.5)
    await drainMain()
    #expect(slot.player.defaultRate == 1.5)
    await slot.unload()
    await drainMain()

    #expect(slot.player.defaultRate == 1)
  }

  @Test
  func unloadFreesTheItemAndTheLoadedKey() async throws {
    let log = PlayerEventLog()
    let slots = LoadedSlots()
    let slot = await makeSlot(log, slots: slots)
    await slot.load(key: "clip", route: route(Self.silentClip))
    await log.wait(for: "ready")
    weak var item = slot.loadedItem
    #expect(slots.slot(for: "clip") === slot)

    await slot.unload()
    await drainMain()

    await eventually("the unloaded item is released") { item == nil }
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
      let slot = await makeSlot(log)
      dropped = slot
      await slot.load(key: "clip", route: route(Self.silentClip))
      await log.wait(for: "ready")
      item = slot.loadedItem
    }
    await eventually("the slot and its item are released") { dropped == nil && item == nil }

    #expect(dropped == nil)
    #expect(item == nil)
  }

  @Test
  func muteAndUnmuteTouchOnlyThePlayerFlag() async throws {
    let slot = await makeSlot(PlayerEventLog())

    await slot.unmute()
    await drainMain()
    #expect(!slot.player.isMuted)
    await slot.mute()
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
    let slot = await makeSlot(log)
    var requests = server.requests.makeAsyncIterator()

    await slot.load(key: "remote", route: route(server.url("/headers"), headers: ["X-Test": "1"]))
    _ = await requests.next()

    #expect(server.headers(of: "/headers")["x-test"] == "1")
  }

  @Test
  func unplayableSourceReportsFailed() async throws {
    let log = PlayerEventLog()
    let slot = await makeSlot(log)

    await slot.load(key: "missing", route: route("file:///nonexistent/clip.mp4"))
    await log.wait(for: "failed 0")

    #expect(log.count("ready") == 0)
  }

  @Test
  func coreAttachPutsTheLayerInTheRegisteredViewAndDetachTakesItOut() async throws {
    let hosts = HostLayers()
    let factory = AVQueuePlayerSlotFactory(startMuted: true, slots: LoadedSlots(), hosts: hosts)
    let slot = await factory.makeSlot(3)
    let host = CALayer()
    let token = hosts.register(host)

    await slot.attach(viewToken: token)
    await drainMain()
    #expect(host.sublayers == [slot.layer])
    #expect(slot.sample().viewAttached)
    await slot.detach()
    await drainMain()

    #expect(host.sublayers == nil)
    #expect(!slot.sample().viewAttached)
  }

  @Test(arguments: [(true, 0), (false, 1)])
  func endOfClipIsReportedOnlyWhenNotLooping(looping: Bool, endedEvents: Int) async throws {
    let log = PlayerEventLog()
    let slot = await makeSlot(log)
    await slot.load(key: "clip", route: route(Self.silentClip, looping: looping))
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
    let slot = await makeSlot(log)
    await slot.load(key: "clip", route: route(Self.silentClip, looping: looping))
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
    let slot = await makeSlot(log)
    await slot.load(key: "clip", route: route(Self.silentClip))
    await log.wait(for: "ready")

    await slot.seek(positionMs: 300)

    await eventually("the playhead reached 0.29 s") { await MainActor.run { slot.player.currentTime().seconds >= 0.29 } }
  }

  @Test
  func inv_idle_05_a_slot_sample_reads_the_live_playhead_without_a_periodic_observer() async {
    let log = PlayerEventLog()
    let slot = await makeSlot(log)
    await slot.load(key: "clip", route: route(Self.silentClip))
    await log.wait(for: "ready")

    await slot.seek(positionMs: 300)

    await eventually("sample() reports the sought position") { slot.sample().positionMs >= 290 }
    await slot.seek(positionMs: 0)
    await eventually("sample() reports the rewind") { slot.sample().positionMs < 50 }
    await slot.play()
    await eventually("sample() advances between player events") { slot.sample().positionMs >= 80 }
    await slot.pause()
    await drainMain()
    let paused = slot.sample().positionMs
    try? await Task.sleep(for: .milliseconds(60))
    #expect(slot.sample().positionMs == paused)
    await slot.setRate(rate: 2)
    await slot.play()
    await eventually("sample() advances after a rate change") { slot.sample().positionMs > paused + 40 }
  }

  @Test
  func aSeekFinishesOnThePlayerBeforeTheNextCommandOfTheLaneRuns() async {
    let log = PlayerEventLog()
    let slot = await makeSlot(log)
    await slot.load(key: "clip", route: route(Self.silentClip))
    await log.wait(for: "ready")
    await slot.play()
    await log.wait(for: "ended")
    await slot.pause()
    #expect(slot.sample().positionMs > 0)

    await slot.seek(positionMs: 0)

    #expect(slot.sample().positionMs == 0)
    await slot.play()
    await eventually("the replay from the start ended again") { log.count("ended") == 2 }
  }

  @Test
  func aSeekAndAPlayIssuedBeforeTheItemIsReadyStartPlaybackFromTheSoughtPosition() async {
    let log = PlayerEventLog()
    let slot = await makeSlot(log)
    await slot.load(key: "clip", route: route(Self.silentClip))

    await slot.seek(positionMs: 300)
    await slot.play()
    await log.wait(for: "ready")

    await eventually("playback runs from the sought position") { slot.sample().positionMs >= 290 }
  }

  @Test
  func aCancelledOrUnloadedSeekOfAnItemThatNeverGetsReadyReturnsAndLeavesTheSlotFree() async {
    let log = PlayerEventLog()
    let slot = await makeSlot(log)
    let stuck = route("http://192.0.2.1/never.mp4")
    await slot.load(key: "a", route: stuck)
    let cancelled = Task { await slot.seek(positionMs: 300) }
    await eventually("the seek waits for the item") { await MainActor.run { slot.player.currentItem?.status == .unknown } }
    cancelled.cancel()
    await cancelled.value

    await slot.load(key: "b", route: stuck)
    let unloaded = Task { await slot.seek(positionMs: 300) }
    try? await Task.sleep(for: .milliseconds(100))
    await slot.unload()
    await unloaded.value

    await slot.load(key: "c", route: route(Self.silentClip))
    await log.wait(for: "ready")
    await slot.seek(positionMs: 300)
    #expect(slot.sample().positionMs >= 290)
  }

  @MainActor
  @Test
  func inv_audio_03_anEndedAudioStopsAtTheStartAndALaterSeekSurvivesARepeatedEnd() {
    let places = RecordingPlaces()
    let audio = AudioPlayback(key: "note", playing: false, places: places)
    audio.toggle()
    audio.refresh(progress: 1)

    audio.follow(.ended)
    #expect(!audio.isPlaying && audio.progress == 0)
    audio.seek(to: 0.5)
    audio.follow(.ended)
    #expect(audio.progress == 0.5)
    audio.toggle()

    #expect(audio.isPlaying)
    #expect(places.recorded.filter { $0.hasPrefix("playing") || $0.hasPrefix("seek") } == ["playing true", "playing false", "seek 0.5", "playing true"])
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
  func inv_audio_03_seekOnAnUnloadedAudioShowsTheFractionAtOnceAndLeavesTheDurationToTheCore() {
    let places = RecordingPlaces()
    let audio = AudioPlayback(key: "note", playing: false, places: places)

    audio.seek(to: 0.5)

    #expect(audio.progress == 0.5)
    #expect(places.recorded == ["seek 0.5"])
  }
}
