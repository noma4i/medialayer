import AVFoundation
import Foundation
import MediaLayer

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
  private let slots: LoadedSlots
  private let hosts: HostLayers

  public init(startMuted: Bool = false, audio: MediaAudioSession? = nil, slots: LoadedSlots = .shared, hosts: HostLayers = .shared) {
    self.startMuted = startMuted
    self.audio = audio
    self.slots = slots
    self.hosts = hosts
  }

  public func create(slot: UInt32) -> PlayerSlot {
    AVQueuePlayerSlot(id: slot, events: events, startMuted: startMuted, audio: audio, slots: slots, hosts: hosts)
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

  private var key: String?
  private var hostKey: String?
  private var item: AVPlayerItem?
  private var observations: [NSKeyValueObservation] = []
  private var traceObservations: [NSKeyValueObservation] = []
  private var audibleObservations: [NSKeyValueObservation] = []
  private var currentItemObservation: NSKeyValueObservation?
  private var notificationTokens: [NSObjectProtocol] = []
  private var prerollRequested = false
  private var visualReadySent = false
  private var itemReady = false
  private var looping = false
  private var wantsPlay = false
  private var readahead: TimeInterval = 0

  public init(
    id: UInt32, events: PlayerEventsLink, startMuted: Bool, audio: MediaAudioSession? = nil, slots: LoadedSlots = .shared,
    hosts: HostLayers = .shared
  ) {
    self.id = id
    self.events = events
    self.audio = audio
    self.slots = slots
    self.hosts = hosts
    super.init()
    onMain { [self] in
      player.isMuted = startMuted
      player.automaticallyWaitsToMinimizeStalling = true
      observeAudibility()
      layer.videoGravity = .resizeAspectFill
      traceStart()
    }
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

  public func load(key: String, route: PlaybackRoute) {
    onMain { [self] in
      guard let url = URL(string: route.url) else {
        events.with { $0.failed(slot: id, status: 0, detail: "invalid url \(route.url)") }
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
      looping = route.looping
      player.actionAtItemEnd = route.looping ? .advance : .pause
      self.key = key
      observe(item)
      player.insert(item, after: nil)
      queueNextRound()
      currentItemObservation = player.observe(\.currentItem) { [weak self] _, _ in
        DispatchQueue.main.async { self?.currentItemChanged() }
      }
      slots.assign(self, key: key)
    }
  }

  public func unload() {
    onMain { [self] in
      let released = key
      MediaTrace.emit("player", "slot=\(id) unload key=\(released.map(MediaTrace.tail) ?? "-")")
      tearDownItem()
      key = nil
      if let released { slots.release(key: released, slot: self) }
    }
  }

  public func preroll() {
    onMain { [self] in
      prerollRequested = true
      prerollIfReady()
    }
  }

  public func play() {
    onMain { [self] in
      wantsPlay = true
      MediaTrace.emit("player", "slot=\(id) play key=\(key.map(MediaTrace.tail) ?? "-")")
      player.play()
    }
  }

  public func pause() {
    onMain { [self] in
      wantsPlay = false
      MediaTrace.emit("player", "slot=\(id) pause key=\(key.map(MediaTrace.tail) ?? "-")")
      player.pause()
    }
  }

  public func mute() {
    onMain { [self] in player.isMuted = true }
  }

  public func unmute() {
    onMain { [self] in player.isMuted = false }
  }

  public func setReadahead(ms: UInt64) {
    onMain { [self] in
      readahead = TimeInterval(ms) / 1000
      player.items().forEach { $0.preferredForwardBufferDuration = readahead }
    }
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

  public func seek(positionMs: UInt64) {
    onMain { [self] in
      player.seek(to: CMTime(value: CMTimeValue(positionMs), timescale: 1000), toleranceBefore: .zero, toleranceAfter: .zero) { [weak self] _ in
        self?.onMain { [weak self] in self?.refreshSample() }
      }
    }
  }

  public func setRate(rate: Float) {
    onMain { [self] in
      player.defaultRate = rate
      if player.rate != 0 { player.rate = rate }
    }
  }

  public func attach(viewToken: UInt64) {
    onMain { [self] in
      guard let host = hosts.layer(for: viewToken) else { return }
      attach(to: host)
    }
  }

  func attach(to host: CALayer) {
    onMain { [self] in
      host.addSublayer(layer)
      hostKey = key
      layer.frame = host.bounds
      refreshSample()
    }
  }

  public func detach() {
    onMain { [self] in
      layer.removeFromSuperlayer()
      hostKey = nil
      refreshSample()
    }
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

  private func observe(_ item: AVPlayerItem, resetState: Bool = true) {
    observations.forEach { $0.invalidate() }
    notificationTokens.forEach { NotificationCenter.default.removeObserver($0) }
    if resetState {
      itemReady = false
      visualReadySent = false
    }
    observations = [
      item.observe(\.status, options: [.initial]) { [weak self] item, _ in self?.onMain { [weak self] in self?.statusChanged(item) } },
      layer.observe(\.isReadyForDisplay, options: [.initial]) { [weak self] _, _ in self?.onMain { [weak self] in self?.displayChanged() } }
    ]
    let center = NotificationCenter.default
    notificationTokens = [
      center.addObserver(forName: .AVPlayerItemDidPlayToEndTime, object: item, queue: .main) { [weak self] _ in
        self?.itemEnded(item)
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
        guard let self else { return }
        let position = positionMs()
        MediaTrace.emit("player", "slot=\(id) stalled key=\(key.map(MediaTrace.tail) ?? "-") position=\(position)")
        events.with { $0.stalled(slot: self.id, positionMs: position) }
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

  private func itemEnded(_ finished: AVPlayerItem) {
    guard finished === item, !looping else { return }

    refreshSample()
    events.with { $0.ended(slot: id) }
  }

  private func currentItemChanged() {
    guard looping, let next = player.currentItem, next !== item else { return }

    item = next
    observe(next, resetState: false)
    queueNextRound()
    refreshSample()
  }

  private func queueNextRound() {
    guard looping, let last = player.items().last, let next = last.copy() as? AVPlayerItem else { return }

    next.preferredForwardBufferDuration = readahead
    player.insert(next, after: last)
  }

  private func statusChanged(_ item: AVPlayerItem) {
    guard item === self.item else { return }
    switch item.status {
    case .readyToPlay:
      MediaTrace.emit("player", "slot=\(id) item ready key=\(key.map(MediaTrace.tail) ?? "-")")
      itemReady = true
      refreshSample()
      events.with { $0.ready(slot: id) }
      prerollIfReady()
    case .failed:
      MediaTrace.emit("player", "slot=\(id) item failed key=\(key.map(MediaTrace.tail) ?? "-") \(item.error.map { String(describing: $0) } ?? "-")")
      events.with { $0.failed(slot: id, status: Self.httpStatus(of: item), detail: item.error?.localizedDescription ?? "failed") }
    default:
      break
    }
  }

  private func displayChanged() {
    guard item != nil, layer.isReadyForDisplay, !visualReadySent else { return }
    visualReadySent = true
    refreshSample()
    events.with { $0.visualReady(slot: id) }
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
      Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] timer in
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

  private func refreshSample() {
    var duration: UInt64?
    if itemReady, let seconds = item?.duration.seconds, seconds.isFinite, seconds > 0 { duration = UInt64(seconds * 1000) }
    let sample = SlotSample(
      positionMs: item == nil ? 0 : positionMs(),
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
