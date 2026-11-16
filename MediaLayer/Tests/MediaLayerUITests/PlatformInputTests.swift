import Foundation
import MediaLayer
import Observation
import Testing
@testable import MediaLayerUI

@Observable
private final class Feed: @unchecked Sendable {
  var rows: [Int] = [1, 2, 3]
  @ObservationIgnored var unrelated = 0
}

private final class Mapped: @unchecked Sendable {
  private let lock = NSLock()
  private(set) var calls = 0
  private(set) var onMain = false

  func item(_ row: Int) -> MediaItem? {
    lock.withLock {
      calls += 1
      if Thread.isMainThread { onMain = true }
    }
    return row == 2 ? nil : MediaItem(kind: .video, url: "https://o.test/\(row).mp4", headers: [:], cover: nil, blurHash: nil, strategy: .window)
  }

  var count: Int { lock.withLock { calls } }
}

private final class GatedPlace: PlaceHandle, @unchecked Sendable {
  func subscribePlayback(listener: any PlaybackListener) throws -> Subscription { Subscription(noHandle: .init()) }
  let entered = DispatchSemaphore(value: 0)
  let gate = DispatchSemaphore(value: 0)
  private let lock = NSLock()
  private var log: [String] = []
  var calls: [String] { lock.withLock { log } }

  func update(input: PlaceInput) throws {
    note("update")
    entered.signal()
    gate.wait()
  }
  func seek(fraction: Double) throws { note("seek \(fraction)") }
  func itemState(state: MediaState) throws {}
  func release() { note("release") }
  func waitForEntry() { entered.wait() }
  private func note(_ call: String) { lock.withLock { log.append(call) }; TestSignal.shared.fire() }
}

@Suite struct PlatformInputTests {
  @Test func inv_ingress_43_aSeekAndACloseInOneDrainWindowSeekThePlaceBeforeItIsReleased() async {
    let place = GatedPlace()
    let places = RecordingPlaces()
    places.placeFactory = { _ in place }
    let mount = PlaceMount(places: places, item: MediaItem(kind: .audio, url: "https://o.test/note.m4a", headers: [:], cover: nil, blurHash: nil, strategy: .window)) { _ in }
    mount.update(.make(visible: true))
    await Task.detached { place.waitForEntry() }.value

    mount.seek(0)
    mount.close()
    place.gate.signal()

    await eventually("the place is released") { place.calls.last == "release" }
    #expect(place.calls == ["update", "seek 0.0", "release"])
  }

  @MainActor
  @Test func aWindowMapsRowsOffTheMainThreadAndAMoveDoesNotPrepareTheList() async {
    let places = RecordingPlaces()
    let feed = Feed()
    let mapped = Mapped()
    let follower = WindowFollower<Int>()
    follower.mount(places: places, layout: .video(source: { mapped.item($0) }), data: { feed.rows }, active: 0, enabled: true, failed: { _ in })
    await eventually("the first list is sent") { places.openedWindows.first?.calls.contains("replace 0 3") == true }
    let prepared = mapped.count

    follower.move(to: 2)
    await eventually("the move is sent") { places.openedWindows.first?.calls.contains("move 2") == true }
    feed.rows = [1, 2, 3, 4]
    await eventually("the new list is sent") { places.openedWindows.first?.calls.contains("replace 2 4") == true }

    #expect(prepared == 3)
    #expect(mapped.count == 7)
    #expect(!mapped.onMain)
    #expect(places.openedWindows.first?.onlyOffMain == true)
    follower.close()
    await eventually("the window is released") { places.openedWindows.first?.calls.last == "release" }
  }

  @MainActor
  @Test func aHeldMappingKeepsOneActiveAndOneLatestPendingWhileAnotherPlaceCloses() async {
    let places = RecordingPlaces()
    let feed = Feed()
    let gate = MappingGate()
    let follower = WindowFollower<Int>()
    follower.mount(places: places, layout: .video(source: { gate.map($0) }), data: { feed.rows }, active: 0, enabled: true, failed: { _ in })
    await eventually("the first mapping is held") { gate.isHeld }

    let other = PlaceMount(places: places, item: MediaItem(kind: .video, url: "other", headers: [:], cover: nil, blurHash: nil, strategy: .window)) { _ in }
    other.update(.make(visible: true))
    await eventually("the other place is shown while the mapping is held") { places.visibleCalls(of: "other") == [true] }
    other.close()
    await eventually("the other place is released while the mapping is held") { places.visibleCalls(of: "other") == [true, false] }
    for round in 0..<500 { feed.rows = [round, round + 1] }
    gate.open()

    await eventually("the latest list is sent") { places.openedWindows.first?.calls.contains("replace 0 2") == true }
    #expect(places.openedWindows.first?.calls.filter { $0.hasPrefix("replace") } == ["replace 0 3", "replace 0 2"])
    #expect(gate.maxActive == 1)
    follower.close()
  }

  @MainActor
  @Test func inv_native_20_open_objects_do_not_share_host_claims() async {
    let first = RecordingPlaces(), second = RecordingPlaces()
    let item = MediaItem(kind: .video, url: "https://o.test/shared.mp4", headers: [:], cover: nil, blurHash: nil, strategy: .window)
    var hostOfFirst: SurfaceHost? = SurfaceHost(places: first)
    let hostOfSecond = SurfaceHost(places: second)
    let tokenOfFirst = hostOfFirst!.token
    let layerOfFirst = hostOfFirst!.layer
    let a = PlaceMount(places: first, item: item) { _ in }
    let b = PlaceMount(places: second, item: item) { _ in }

    a.update(.make(visible: true, viewToken: tokenOfFirst))
    b.update(.make(visible: true, viewToken: hostOfSecond.token))
    await eventually("both objects attach the view of the same url") {
      first.recorded.contains("attach https://o.test/shared.mp4") && second.recorded.contains("attach https://o.test/shared.mp4")
    }
    #expect(HostLayers.of(first).layer(for: tokenOfFirst) === layerOfFirst)
    #expect(HostLayers.of(second).layer(for: hostOfSecond.token) === hostOfSecond.layer)
    #expect(hostOfSecond.layer !== layerOfFirst)
    #expect(HostLayers.of(second).layer(for: tokenOfFirst) !== layerOfFirst)
    #expect(HostLayers.of(first).layer(for: hostOfSecond.token) !== hostOfSecond.layer)

    a.close()
    hostOfFirst = nil
    await eventually("the first object released its place") { first.visibleCalls(of: item.url) == [true, false] }
    #expect(HostLayers.of(first).layer(for: tokenOfFirst) == nil)
    #expect(HostLayers.of(second).layer(for: hostOfSecond.token) === hostOfSecond.layer)
    #expect(second.visibleCalls(of: item.url) == [true])
    b.close()
  }

  @MainActor
  @Test func aNilContextClosesTheWindowThatWasMounted() async {
    let places = RecordingPlaces()
    let follower = WindowFollower<Int>()
    follower.mount(places: places, layout: .video(source: { _ in nil }), data: { [1] }, active: 0, enabled: true, failed: { _ in })
    await eventually("opened") { places.openedWindows.first?.calls.contains("replace 0 1") == true }

    follower.close()

    await eventually("released") { places.openedWindows.first?.calls.last == "release" }
  }

  @Test func aPlaceMountAppliesTheFirstInputThenReleasesOnCloseWhateverFollowed() async {
    let places = RecordingPlaces()
    let mount = PlaceMount(places: places, item: MediaItem(kind: .video, url: "k", headers: [:], cover: nil, blurHash: nil, strategy: .window)) { _ in }
    mount.update(.make(visible: true, playing: true))
    await eventually("the first input is applied") { places.visibleCalls(of: "k") == [true] }

    for index in 0..<1_000 { mount.update(.make(visible: true, playing: index % 2 == 0)) }
    mount.close()

    await eventually("released") { places.visibleCalls(of: "k") == [true, false] }
    #expect(places.recorded.filter { $0 == "mount" }.count == 1)
  }
}

private final class MappingGate: @unchecked Sendable {
  private let lock = NSLock()
  private let semaphore = DispatchSemaphore(value: 0)
  private var first = true
  private var held = false
  private var active = 0
  private(set) var maxActive = 0

  var isHeld: Bool { lock.withLock { held } }

  func map(_ row: Int) -> MediaItem? {
    let blocks = lock.withLock { () -> Bool in
      active += 1
      maxActive = max(maxActive, active)
      defer { first = false }
      if first { held = true }
      return first
    }
    TestSignal.shared.fire()
    if blocks { semaphore.wait() }
    lock.withLock { active -= 1 }
    return nil
  }

  func open() { semaphore.signal() }
}

private final class RefusingPlace: PlaceHandle, @unchecked Sendable {
  func subscribePlayback(listener: any PlaybackListener) throws -> Subscription { Subscription(noHandle: .init()) }
  private let lock = NSLock()
  private var attempts = 0
  private var released = 0
  var updates: Int { lock.withLock { attempts } }
  var releases: Int { lock.withLock { released } }

  func update(input: PlaceInput) throws {
    lock.withLock { attempts += 1 }
    TestSignal.shared.fire()
    throw MediaError.Overloaded(detail: "entry limit reached")
  }

  func seek(fraction: Double) throws {}
  func itemState(state: MediaState) throws {}
  func release() { lock.withLock { released += 1 }; TestSignal.shared.fire() }
}

extension PlatformInputTests {
  @Test func aCloseMergedWithARefusedUpdateStillReleasesTheCorePlace() async {
    let places = RecordingPlaces()
    let refusing = RefusingPlace()
    places.placeFactory = { _ in refusing }
    let mount = PlaceMount(places: places, item: MediaItem(kind: .video, url: "k", headers: [:], cover: nil, blurHash: nil, strategy: .window)) { _ in }
    mount.update(.make(visible: true))
    await eventually("the refused update was tried") { refusing.updates == 1 }

    let held = HeldQueue()
    mount.update(.make(visible: true, playing: true))
    mount.close()
    mount.close()
    held.release()

    await eventually("released once") { refusing.releases == 1 }
    #expect(refusing.updates == 1)
  }

  @Test func aCloseBeforeTheFirstMountNeverMountsAnything() async {
    let places = RecordingPlaces()
    let mount = PlaceMount(places: places, item: MediaItem(kind: .video, url: "never", headers: [:], cover: nil, blurHash: nil, strategy: .window)) { _ in }

    mount.close()
    mount.update(.make(visible: true))
    await mount.idle()

    #expect(places.recorded.filter { $0 == "mount" }.isEmpty)
  }
}

extension PlatformInputTests {
  @Test func theIdentityOfAMountFollowsTheFullItemAndTheContext() {
    let item = MediaItem(kind: .video, url: "k", headers: [:], cover: nil, blurHash: nil, strategy: .window)
    var other = item
    other.headers = ["X": "1"]
    var covered = item
    covered.cover = "c"
    let first = MediaContext(places: RecordingPlaces()), second = MediaContext(places: RecordingPlaces())

    #expect(MountIdentity(item: item, context: first) == MountIdentity(item: item, context: first))
    #expect(MountIdentity(item: item, context: first) != MountIdentity(item: other, context: first))
    #expect(MountIdentity(item: item, context: first) != MountIdentity(item: covered, context: first))
    #expect(MountIdentity(item: item, context: first) != MountIdentity(item: item, context: second))
    #expect(MountIdentity(item: item, context: first) != MountIdentity(item: item, context: nil))
  }

  @Test func aRefusedMountReachesTheOwnerAsAFailure() async {
    let places = RecordingPlaces()
    places.mountError = MediaError.Overloaded(detail: "owner limit reached")
    let failures = FailureLog()
    let mount = PlaceMount(places: places, item: MediaItem(kind: .video, url: "k", headers: [:], cover: nil, blurHash: nil, strategy: .window)) { failures.add($0) }

    mount.update(.make(visible: true))

    await eventually("the failure arrives") { failures.count == 1 }
    mount.close()
  }

  @MainActor
  @Test func aRefusedMountMakesAnAudioFailed() async {
    let places = RecordingPlaces()
    places.mountError = MediaError.Overloaded(detail: "owner limit reached")
    let audio = AudioPlayback(item: MediaItem(kind: .audio, url: "note", headers: [:], cover: nil, blurHash: nil, strategy: .window), playing: true, places: places)

    audio.mountPlace()

    await eventually("failed") { if case .failed = audio.state { true } else { false } }
  }

  @MainActor
  @Test func aRefusedGlobalMuteThrowsAndLeavesTheFlagAndReportsTheFailure() {
    let places = RecordingPlaces()
    Media.current = places
    defer { Media.current = nil }
    let before = Media.muted
    places.mutedAllError = MediaError.Closed

    #expect(throws: MediaError.self) { try Media.setMuted(!before) }

    #expect(Media.muted == before)
    #expect(Media.failure != nil)
    places.mutedAllError = nil
    try? Media.setMuted(before)
    #expect(Media.failure == nil)
  }

  @Test func anImageRecorderDoesNotKeepItsCountersAlive() async {
    weak var weakCounters: ImageCounters?
    do {
      let counters = ImageCounters()
      signalOnRelease(counters)
      weakCounters = counters
      counters.recordLater(work: .observe)
      await eventually("flushed") { counters.snapshot().activeObservers == 1 }
    }

    await eventually("released") { weakCounters == nil }
  }
}

private final class FailureLog: @unchecked Sendable {
  private let lock = NSLock()
  private var errors: [MediaError] = []
  var count: Int { lock.withLock { errors.count } }
  func add(_ error: MediaError) { lock.withLock { errors.append(error) }; TestSignal.shared.fire() }
}

extension PlatformInputTests {
  @MainActor
  @Test func anotherModelMountsANewWindowAndTheOldModelNoLongerArrives() async {
    let places = RecordingPlaces()
    let first = Feed(), second = Feed()
    second.rows = [7, 8]
    let follower = WindowFollower<Int>()
    follower.mount(places: places, layout: .video(source: { _ in nil }), data: { first.rows }, active: 0, enabled: true, failed: { _ in })
    await eventually("the first model is sent") { places.openedWindows.first?.calls.contains("replace 0 3") == true }

    follower.mount(places: places, layout: .video(source: { _ in nil }), data: { second.rows }, active: 0, enabled: true, failed: { _ in })
    await eventually("the new model is sent to a new window") { places.openedWindows.count == 2 && places.openedWindows[1].calls.contains("replace 0 2") }
    first.rows = [9]
    second.rows = [1, 2, 3, 4]
    await eventually("the new model follows") { places.openedWindows[1].calls.contains("replace 0 4") }

    #expect(places.openedWindows[0].calls.last == "release")
    #expect(!places.openedWindows[1].calls.contains("replace 0 1"))
    follower.close()
  }

  @MainActor
  @Test func theLatestMapperServesTheNextSendAndTheOldOneIsReleased() async {
    let places = RecordingPlaces()
    let feed = Feed()
    let follower = WindowFollower<Int>()
    weak var weakProbe: MapperProbe?
    do {
      let probe = MapperProbe()
      signalOnRelease(probe)
      weakProbe = probe
      follower.mount(places: places, layout: .video(source: { _ in probe.touch(); return nil }), data: { feed.rows }, active: 0, enabled: true, failed: { _ in })
    }
    await eventually("the first list is sent") { places.openedWindows.first?.calls.contains("replace 0 3") == true }
    let next = MapperProbe()

    follower.use(layout: .video(source: { _ in next.touch(); return nil }))

    await eventually("the replaced mapper is released") { weakProbe == nil }
    feed.rows = [1]
    await eventually("the next send uses the latest mapper") { places.openedWindows.first?.calls.contains("replace 0 1") == true }
    #expect(next.touches == 1)
    follower.close()
  }

  @Test func aFailureOfAnotherItemOfTheSameUrlIsNotInherited() {
    let old = MediaItem(kind: .image, url: "k", headers: [:], cover: nil, blurHash: nil, strategy: .window)
    var renewed = old
    renewed.headers = ["X": "1"]
    let failure = ScopedFailure(item: old, state: .failed(detail: "x"))

    #expect(failure.of(old) != nil)
    #expect(failure.of(renewed) == nil)
  }
}

private final class MapperProbe: @unchecked Sendable {
  private let lock = NSLock()
  private var count = 0
  var touches: Int { lock.withLock { count } }
  func touch() { lock.withLock { count += 1 } }
}

private final class Readings: @unchecked Sendable {
  private let lock = NSLock()
  private var values: [Bool] = []

  func add(_ value: Bool) { lock.withLock { values.append(value) }; TestSignal.shared.fire() }

  var all: [Bool] { lock.withLock { values } }
}

extension PlatformInputTests {
  @MainActor
  @Test func inv_window_05_theReadinessOfTheCoreReachesTheViewAndAClosedWindowIsNotReady() async {
    let places = RecordingPlaces()
    let feed = Feed()
    let readings = Readings()
    let follower = WindowFollower<Int>()
    follower.use(ready: { readings.add($0) })
    follower.mount(
      places: places, layout: .image(name: "grid", rowsPerPage: 1, reversed: false, sources: { _ in [] }), data: { feed.rows }, active: 0, enabled: true, failed: { _ in }
    )
    await eventually("the window is opened") { places.openedWindows.first?.calls.contains("rows 3") == true }

    places.openedWindows[0].reportReady(true)
    await eventually("the core readiness arrives") { readings.all == [true] }
    follower.close()

    #expect(readings.all == [true, false])
  }

  @MainActor
  @Test func inv_window_06_theVisibleRowsOfTheListMoveTheWindowWithoutAnActivePage() async {
    let places = RecordingPlaces()
    let feed = Feed()
    let follower = WindowFollower<Int>()
    follower.mount(
      places: places, layout: .image(name: "grid", rowsPerPage: 2, reversed: true, sources: { _ in [] }), data: { feed.rows }, active: 0, enabled: true, tracksRows: true,
      failed: { _ in }
    )
    await eventually("the rows are sent") { places.openedWindows.first?.calls.contains("rows 3") == true }

    follower.show(1...2)
    await eventually("the visible rows are sent") { places.openedWindows.first?.calls.contains("show 1 2") == true }

    #expect(places.openedWindows.first?.calls.contains { $0.hasPrefix("move") } == false)
    follower.close()
  }
}
