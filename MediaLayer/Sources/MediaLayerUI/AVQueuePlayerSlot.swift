import AVFoundation
import Foundation
import MediaLayer

private final class Latch: @unchecked Sendable {
  private let lock = NSLock()
  private var value = false

  var isSet: Bool { lock.withLock { value } }

  func set() { lock.withLock { value = true } }
}

public final class PlayerEventsLink: @unchecked Sendable {
  private let lock = NSLock()
  private var target: PlayerEventsProtocol?

  public init() {}

  public func bind(_ events: PlayerEventsProtocol?) {
    lock.withLock { target = events }
  }

  func with(_ body: (PlayerEventsProtocol) -> Void) {
    guard let events = lock.withLock({ target }) else { return }
    body(events)
  }
}

public final class AVQueuePlayerSlotFactory: PlayerSlotFactory, @unchecked Sendable {
  public let events = PlayerEventsLink()
  private let startMuted: Bool
  private let audio: MediaAudioSession?
  let slots: LoadedSlots
  let hosts: HostLayers

  public init(startMuted: Bool = false, audio: MediaAudioSession? = nil, slots: LoadedSlots = LoadedSlots(), hosts: HostLayers = HostLayers()) {
    self.startMuted = startMuted
    self.audio = audio
    self.slots = slots
    self.hosts = hosts
  }

  public func create(slot: UInt32) async -> CreatedPlayer {
    let created = await MainActor.run {
      AVQueuePlayerSlot(id: slot, events: events, startMuted: startMuted, audio: audio, slots: slots, hosts: hosts)
    }
    return CreatedPlayer(player: created)
  }

  func release() {
    events.bind(nil)
  }
}

class SeekWait: @unchecked Sendable {
  let positionMs: UInt64
  var item: AVPlayerItem?
  var isStarted = false
  private let lock = NSLock()
  private var continuation: CheckedContinuation<Void, Never>?
  private var finished = false

  init(positionMs: UInt64) { self.positionMs = positionMs }

  var isFinished: Bool { lock.withLock { finished } }

  func cancelNative() { item?.cancelPendingSeeks() }

  func park(_ parked: CheckedContinuation<Void, Never>) {
    let isDone = lock.withLock { () -> Bool in
      if !finished { continuation = parked }
      return finished
    }
    if isDone { parked.resume() }
  }

  func finish() {
    let parked = lock.withLock { () -> CheckedContinuation<Void, Never>? in
      finished = true
      defer { continuation = nil }
      return continuation
    }
    parked?.resume()
  }
}

public final class AVQueuePlayerSlot: NSObject, PlayerSlot, @unchecked Sendable {
  public let id: UInt32
  public private(set) lazy var player: AVQueuePlayer = {
    dispatchPrecondition(condition: .onQueue(.main))
    return AVQueuePlayer()
  }()
  public private(set) lazy var layer: AVPlayerLayer = {
    dispatchPrecondition(condition: .onQueue(.main))
    return AVPlayerLayer(player: player)
  }()

  private let events: PlayerEventsLink
  private let audio: MediaAudioSession?
  private let slots: LoadedSlots
  private let hosts: HostLayers
  private let sampleLock = NSLock()
  private var cachedSample = SlotSample(positionMs: 0, durationMs: nil, isLooping: false, viewAttached: false, frameRendered: false)
  private var sampledAt: TimeInterval = 0
  private var sampledRate: Double = 0
  private var commanded = Commanded()

  private var key: String?
  private var hostKey: String?
  private var item: AVPlayerItem?
  private var observations: [NSKeyValueObservation] = []
  private var traceObservations: [NSKeyValueObservation] = []
  private var audibleObservations: [NSKeyValueObservation] = []
  private var currentItemObservation: NSKeyValueObservation?
  private var notificationTokens: [NSObjectProtocol] = []
  private var prerollRequested = false
  private var loadGeneration: UInt64 = 0
  private var visualReadySent = false
  private var itemReady = false
  private var looping = false
  private var wantsPlay = false
  private var seekWait: SeekWait?
  var isSeeking: Bool { seekWait != nil }
  private var readahead: TimeInterval = 0
  private let closed = Latch()
  private var isDisposed = false
  private var traceTimer: Timer?

  public init(
    id: UInt32, events: PlayerEventsLink, startMuted: Bool, audio: MediaAudioSession? = nil, slots: LoadedSlots = LoadedSlots(),
    hosts: HostLayers = HostLayers()
  ) {
    self.id = id
    self.events = events
    self.audio = audio
    self.slots = slots
    self.hosts = hosts
    super.init()
    dispatchPrecondition(condition: .onQueue(.main))
    player.isMuted = startMuted
    sampleLock.withLock { commanded.isMuted = startMuted }
    player.automaticallyWaitsToMinimizeStalling = true
    observeAudibility()
    layer.videoGravity = .resizeAspectFill
    traceStart()
  }

  deinit {
    audio?.setAudible(slot: id, false)
    let player = player
    let layer = layer
    let observations = observations + audibleObservations + traceObservations + [currentItemObservation].compactMap { $0 }
    let tokens = notificationTokens
    DispatchQueue.main.async {
      observations.forEach { $0.invalidate() }
      tokens.forEach { NotificationCenter.default.removeObserver($0) }
      player.removeAllItems()
      layer.removeFromSuperlayer()
    }
  }

  public func apply(command: SlotCall) async {
    if case let .seek(positionMs) = command {
      await applySeek(positionMs: positionMs)
      return
    }
    await MainActor.run { [self] in
      guard !closed.isSet, !Task.isCancelled else { return }
      perform(command)
    }
  }

  public func dispose() async {
    closed.set()
    await MainActor.run { [self] in disposeNow() }
  }

  func close() {
    closed.set()
    DispatchQueue.main.async { [self] in disposeNow() }
  }

  private func disposeNow() {
    dispatchPrecondition(condition: .onQueue(.main))
    guard !isDisposed else { return }
    isDisposed = true
    unload()
    detach()
    (audibleObservations + traceObservations).forEach { $0.invalidate() }
    audibleObservations = []
    traceObservations = []
    traceTimer?.invalidate()
    traceTimer = nil
    audio?.setAudible(slot: id, false)
  }

  private func perform(_ command: SlotCall) {
    switch command {
    case let .load(key, route, generation): load(key: key, route: route, generation: generation)
    case .unload: unload()
    case .preroll: preroll()
    case .play: play()
    case .pause: pause()
    case .mute: player.isMuted = true; sampleLock.withLock { commanded.isMuted = true }
    case .unmute: player.isMuted = false; sampleLock.withLock { commanded.isMuted = false }
    case let .attach(viewToken): attach(viewToken: viewToken)
    case .detach: detach()
    case .seek: break  // applySeek waits for the completion of a seek, perform cannot
    case let .rate(rate): setRate(rate: rate)
    case let .readahead(ms): setReadahead(ms: ms)
    }
  }

  private func load(key: String, route: PlaybackRoute, generation: UInt64) {
    dispatchPrecondition(condition: .onQueue(.main))
    guard let url = URL(string: route.url) else {
      events.with { [id] in $0.failed(slot: id, generation: generation, status: 0, detail: "invalid url \(route.url)") }
      return
    }
    MediaTrace.emit("player", "slot=\(id) load key=\(MediaTrace.tail(key)) url=\(MediaTrace.tail(route.url))")
    tearDownItem()
    if hostKey != key {
      layer.removeFromSuperlayer()
      hostKey = nil
    }
    let item = AVPlayerItem(asset: AVURLAsset(url: url, options: Self.assetOptions(for: route)))
    self.item = item
    loadGeneration = generation
    looping = route.looping
    player.actionAtItemEnd = route.looping ? .advance : .pause
    self.key = key
    sampleLock.withLock { commanded.key = key }
    observe(item, generation: generation)
    player.insert(item, after: nil)
    queueNextRound()
    currentItemObservation = player.observe(\.currentItem) { [weak self] _, _ in
      DispatchQueue.main.async { self?.currentItemChanged(generation: generation) }
    }
    slots.assign(self, key: key)
  }

  private func unload() {
    dispatchPrecondition(condition: .onQueue(.main))
    let released = key
    MediaTrace.emit("player", "slot=\(id) unload key=\(released.map(MediaTrace.tail) ?? "-")")
    tearDownItem()
    key = nil
    sampleLock.withLock { commanded.key = nil; commanded.isPlaying = false; commanded.rate = 1 }
    if let released { slots.release(key: released, slot: self) }
  }

  private func preroll() {
    prerollRequested = true
    prerollIfReady()
  }

  private func play() {
    wantsPlay = true
    sampleLock.withLock { commanded.isPlaying = true }
    MediaTrace.emit("player", "slot=\(id) play key=\(key.map(MediaTrace.tail) ?? "-")")
    player.play()
  }

  private func pause() {
    wantsPlay = false
    sampleLock.withLock { commanded.isPlaying = false }
    MediaTrace.emit("player", "slot=\(id) pause key=\(key.map(MediaTrace.tail) ?? "-")")
    player.pause()
  }

  private func setReadahead(ms: UInt64) {
    readahead = TimeInterval(ms) / 1000
    player.items().forEach { $0.preferredForwardBufferDuration = readahead }
  }

  public func sample() -> SlotSample {
    sampleLock.withLock {
      var sample = cachedSample
      if sampledRate > 0 {
        sample.positionMs += UInt64(max(0, ProcessInfo.processInfo.systemUptime - sampledAt) * sampledRate * 1000)
        if let duration = sample.durationMs { sample.positionMs = min(sample.positionMs, duration) }
      }
      return sample
    }
  }

  public func snapshot() -> PlayerSnapshot? {
    let state = sampleLock.withLock { commanded }
    guard let key = state.key else { return nil }
    return PlayerSnapshot(key: key, isPlaying: state.isPlaying, isMuted: state.isMuted, positionMs: sample().positionMs, rate: state.rate)
  }

  private func applySeek(positionMs: UInt64) async {
    let wait = SeekWait(positionMs: positionMs)
    await withTaskCancellationHandler {
      await withCheckedContinuation { (continuation: CheckedContinuation<Void, Never>) in
        wait.park(continuation)
        Task { @MainActor [self] in beginSeek(wait) }
      }
    } onCancel: {
      Task { @MainActor [self] in cancelSeek(wait) }
    }
  }

  func beginSeek(_ wait: SeekWait) {
    dispatchPrecondition(condition: .onQueue(.main))
    guard !wait.isFinished, !closed.isSet, let item else { return wait.finish() }
    finishSeek()
    wait.item = item
    seekWait = wait
    if item.status == .readyToPlay { startSeek(wait) }
  }

  private func startSeek(_ wait: SeekWait) {
    wait.isStarted = true
    player.seek(to: CMTime(value: CMTimeValue(wait.positionMs), timescale: 1000), toleranceBefore: .zero, toleranceAfter: .zero) { [weak self] _ in
      self?.onMain { [weak self] in
        self?.refreshSample()
        self?.completeSeek(wait)
      }
    }
  }

  private func completeSeek(_ wait: SeekWait) {
    wait.finish()
    if seekWait === wait { seekWait = nil }
    if let key { Task { @MainActor [slots] in slots.progress.changed(key) } }
  }

  func cancelSeek(_ wait: SeekWait) {
    if seekWait === wait { wait.cancelNative() }
    completeSeek(wait)
  }

  private func finishSeek() {
    guard let wait = seekWait else { return }
    seekWait = nil
    wait.finish()
  }

  private func setRate(rate: Float) {
    player.defaultRate = rate
    sampleLock.withLock { commanded.rate = rate }
    if player.rate != 0 { player.rate = rate }
  }

  private func attach(viewToken: UInt64) {
    guard let host = hosts.layer(for: viewToken) else { return }
    attach(to: host)
  }

  func attach(to host: CALayer) {
    dispatchPrecondition(condition: .onQueue(.main))
    host.addSublayer(layer)
    hostKey = key
    layer.frame = host.bounds
    refreshSample()
  }

  private func detach() {
    layer.removeFromSuperlayer()
    hostKey = nil
    refreshSample()
  }

  static func assetOptions(for route: PlaybackRoute) -> [String: Any]? {
    var options: [String: Any] = [:]
    if !route.headers.isEmpty { options["AVURLAssetHTTPHeaderFieldsKey"] = route.headers }
    if route.hls { options["AVURLAssetOverrideMIMETypeKey"] = "application/x-mpegURL" }
    return options.isEmpty ? nil : options
  }

  var loadedItem: AVPlayerItem? { item }

  private func onMain(_ work: @escaping () -> Void) {
    DispatchQueue.main.async(execute: work)
  }

  private func observe(_ item: AVPlayerItem, generation: UInt64, resetState: Bool = true) {
    observations.forEach { $0.invalidate() }
    notificationTokens.forEach { NotificationCenter.default.removeObserver($0) }
    if resetState {
      itemReady = false
      visualReadySent = false
    }
    observations = [
      item.observe(\.status, options: [.initial]) { [weak self] item, _ in self?.onMain { [weak self] in self?.statusChanged(item, generation: generation) } },
      layer.observe(\.isReadyForDisplay, options: [.initial]) { [weak self] _, _ in self?.onMain { [weak self] in self?.displayChanged(item, generation: generation) } }
    ]
    let center = NotificationCenter.default
    notificationTokens = [
      center.addObserver(forName: .AVPlayerItemDidPlayToEndTime, object: item, queue: .main) { [weak self] _ in
        self?.itemEnded(item, generation: generation)
      },
      center.addObserver(forName: .AVPlayerItemNewErrorLogEntry, object: item, queue: .main) { [weak self] _ in
        guard let self, MediaTrace.isOn, let event = item.errorLog()?.events.last else { return }
        MediaTrace.emit("player", "slot=\(id) errorlog status=\(event.errorStatusCode) domain=\(event.errorDomain) uri=\(event.uri.map(MediaTrace.tail) ?? "-") comment=\(event.errorComment ?? "-")")
      },
      center.addObserver(forName: .AVPlayerItemFailedToPlayToEndTime, object: item, queue: .main) { [weak self] note in
        guard let self else { return }
        MediaTrace.emit("player", "slot=\(id) failed-to-end key=\(key.map(MediaTrace.tail) ?? "-") \(String(describing: note.userInfo?[AVPlayerItemFailedToPlayToEndTimeErrorKey]))")
      },
      center.addObserver(forName: .AVPlayerItemPlaybackStalled, object: item, queue: .main) { [weak self] _ in
        guard let self, item === self.item, generation == loadGeneration else { return }
        let position = positionMs()
        MediaTrace.emit("player", "slot=\(id) stalled key=\(key.map(MediaTrace.tail) ?? "-") position=\(position)")
        events.with { $0.stalled(slot: self.id, generation: generation, positionMs: position) }
      }
    ]
  }

  private func observeAudibility() {
    let report: (AVPlayer) -> Void = { [weak self] player in
      self?.onMain { [weak self] in
        guard let self else { return }
        refreshSample()
        audio?.setAudible(slot: id, !player.isMuted && player.timeControlStatus != .paused && player.currentItem != nil)
      }
    }
    audibleObservations = [
      player.observe(\.isMuted, options: [.new]) { player, _ in report(player) },
      player.observe(\.rate, options: [.new]) { player, _ in report(player) },
      player.observe(\.timeControlStatus, options: [.new]) { player, _ in report(player) },
      player.observe(\.currentItem, options: [.new]) { player, _ in report(player) }
    ]
  }

  func itemEnded(_ finished: AVPlayerItem, generation: UInt64) {
    guard finished === item, generation == loadGeneration, !looping else { return }

    refreshSample(atEnd: true)
    events.with { $0.ended(slot: id, generation: generation) }
  }

  private func currentItemChanged(generation: UInt64) {
    guard generation == loadGeneration, looping, let next = player.currentItem, next !== item else { return }

    item = next
    observe(next, generation: generation, resetState: false)
    queueNextRound()
    refreshSample()
  }

  private func queueNextRound() {
    guard looping, let last = player.items().last, let next = last.copy() as? AVPlayerItem else { return }

    next.preferredForwardBufferDuration = readahead
    player.insert(next, after: last)
  }

  func statusChanged(_ item: AVPlayerItem, generation: UInt64) {
    guard item === self.item, generation == loadGeneration else { return }
    switch item.status {
    case .readyToPlay:
      MediaTrace.emit("player", "slot=\(id) item ready key=\(key.map(MediaTrace.tail) ?? "-")")
      itemReady = true
      if let wait = seekWait, !wait.isStarted, wait.item === item { startSeek(wait) }
      refreshSample()
      events.with { $0.ready(slot: id, generation: generation) }
      prerollIfReady()
    case .failed:
      finishSeek()
      MediaTrace.emit("player", "slot=\(id) item failed key=\(key.map(MediaTrace.tail) ?? "-") \(item.error.map { String(describing: $0) } ?? "-")")
      events.with { $0.failed(slot: id, generation: generation, status: Self.httpStatus(of: item), detail: item.error?.localizedDescription ?? "failed") }
    default:
      break
    }
  }

  private func displayChanged(_ shown: AVPlayerItem, generation: UInt64) {
    guard shown === item, generation == loadGeneration, layer.isReadyForDisplay, !visualReadySent else { return }
    visualReadySent = true
    refreshSample()
    events.with { $0.visualReady(slot: id, generation: generation) }
  }

  private func prerollIfReady() {
    guard prerollRequested, itemReady, player.status == .readyToPlay, player.rate == 0 else { return }
    prerollRequested = false
    player.preroll(atRate: 1) { _ in }
  }

  private func tearDownItem() {
    currentItemObservation?.invalidate()
    currentItemObservation = nil
    observations.forEach { $0.invalidate() }
    observations = []
    notificationTokens.forEach { NotificationCenter.default.removeObserver($0) }
    notificationTokens = []
    item?.asset.cancelLoading()
    player.pause()
    player.removeAllItems()
    item = nil
    player.defaultRate = 1
    prerollRequested = false
    finishSeek()
    itemReady = false
    visualReadySent = false
    readahead = 0
    looping = false
    refreshSample()
  }

  private func traceStart() {
    guard MediaTrace.isOn else { return }
    traceObservations = [
      player.observe(\.rate, options: [.new]) { [weak self] player, _ in
        guard let self else { return }
        MediaTrace.emit("player", "slot=\(id) rate=\(player.rate) key=\(key.map(MediaTrace.tail) ?? "-")")
      },
      player.observe(\.timeControlStatus, options: [.new]) { [weak self] player, _ in
        guard let self else { return }
        let reason = player.reasonForWaitingToPlay?.rawValue ?? "-"
        MediaTrace.emit("player", "slot=\(id) control=\(player.timeControlStatus.rawValue) reason=\(reason) key=\(key.map(MediaTrace.tail) ?? "-")")
      }
    ]
    DispatchQueue.main.async { [weak self] in
      guard let self, !isDisposed else { return }
      traceTimer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] timer in
        guard let self else { return timer.invalidate() }
        traceSnapshot()
      }
    }
  }

  private func traceSnapshot() {
    guard key != nil || player.rate != 0 else { return }
    let buffered = item?.loadedTimeRanges.first?.timeRangeValue.end.seconds ?? 0
    let seconds = player.currentTime().seconds
    MediaTrace.emit(
      "snapshot",
      "slot=\(id) key=\(key.map(MediaTrace.tail) ?? "-") rate=\(player.rate) control=\(player.timeControlStatus.rawValue) "
        + "status=\(item?.status.rawValue ?? -1) time=\(String(format: "%.2f", seconds.isFinite ? seconds : 0)) buffered=\(String(format: "%.2f", buffered)) "
        + "display=\(layer.isReadyForDisplay) attached=\(layer.superlayer != nil) wants=\(wantsPlay)"
    )
  }

  private func positionMs() -> UInt64 {
    let seconds = player.currentTime().seconds
    return seconds.isFinite && seconds > 0 ? UInt64(seconds * 1000) : 0
  }

  private func refreshSample(atEnd: Bool = false) {
    var duration: UInt64?
    if itemReady, let seconds = item?.duration.seconds, seconds.isFinite, seconds > 0 { duration = UInt64(seconds * 1000) }
    let sample = SlotSample(
      positionMs: atEnd ? duration ?? positionMs() : item == nil ? 0 : positionMs(),
      durationMs: duration,
      isLooping: looping,
      viewAttached: layer.superlayer != nil,
      frameRendered: item != nil && layer.isReadyForDisplay
    )
    let rate = player.timeControlStatus == .playing ? Double(player.rate) : 0
    sampleLock.withLock {
      cachedSample = sample
      sampledAt = ProcessInfo.processInfo.systemUptime
      sampledRate = rate
    }
    if let key { Task { @MainActor [slots] in slots.progress.changed(key) } }
  }

  private static func httpStatus(of item: AVPlayerItem) -> Int32 {
    Int32(item.errorLog()?.events.last?.errorStatusCode ?? 0)
  }
}

private struct Commanded {
  var key: String?
  var isPlaying = false
  var isMuted = false
  var rate: Float = 1
}
