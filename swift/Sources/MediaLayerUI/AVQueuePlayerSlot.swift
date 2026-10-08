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
  private let slots: LoadedSlots
  private let hosts: HostLayers

  public init(startMuted: Bool = false, slots: LoadedSlots = .shared, hosts: HostLayers = .shared) {
    self.startMuted = startMuted
    self.slots = slots
    self.hosts = hosts
  }

  public func create(slot: UInt32) -> PlayerSlot {
    AVQueuePlayerSlot(id: slot, events: events, startMuted: startMuted, slots: slots, hosts: hosts)
  }
}

public final class AVQueuePlayerSlot: NSObject, PlayerSlot, @unchecked Sendable {
  public static let warmBuffer: TimeInterval = 2
  public static let coldBuffer: TimeInterval = 0.5
  private static let sampleInterval = CMTime(value: 1, timescale: 4)

  public let id: UInt32
  public let player = AVQueuePlayer()
  public let layer: AVPlayerLayer

  private let events: PlayerEventsLink
  private let slots: LoadedSlots
  private let hosts: HostLayers
  private let sampleLock = NSLock()
  private var cachedSample = SlotSample(positionMs: 0, durationMs: nil, isLooping: false, viewAttached: false, frameRendered: false)

  private var key: String?
  private var item: AVPlayerItem?
  private var observations: [NSKeyValueObservation] = []
  private var notificationTokens: [NSObjectProtocol] = []
  private var timeToken: Any?
  private var prerollRequested = false
  private var visualReadySent = false
  private var itemReady = false
  private var looping = false
  private var wantsPlay = false

  public init(id: UInt32, events: PlayerEventsLink, startMuted: Bool, slots: LoadedSlots = .shared, hosts: HostLayers = .shared) {
    self.id = id
    self.events = events
    self.slots = slots
    self.hosts = hosts
    layer = AVPlayerLayer(player: player)
    super.init()
    player.isMuted = startMuted
    player.automaticallyWaitsToMinimizeStalling = true
    layer.videoGravity = .resizeAspectFill
  }

  deinit {
    let player = player
    let observations = observations
    let tokens = notificationTokens
    let timeToken = timeToken
    DispatchQueue.main.async {
      observations.forEach { $0.invalidate() }
      tokens.forEach { NotificationCenter.default.removeObserver($0) }
      if let timeToken { player.removeTimeObserver(timeToken) }
      player.removeAllItems()
    }
  }

  public func load(key: String, route: PlaybackRoute) {
    onMain { [self] in
      guard let url = URL(string: route.url) else {
        events.with { $0.failed(slot: id, status: 0, detail: "invalid url \(route.url)") }
        return
      }
      tearDownItem()
      let item = AVPlayerItem(asset: AVURLAsset(url: url, options: Self.assetOptions(for: route)))
      self.item = item
      looping = route.looping
      self.key = key
      observe(item)
      player.insert(item, after: nil)
      slots.assign(self, key: key)
    }
  }

  public func unload() {
    onMain { [self] in
      let released = key
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
      player.play()
    }
  }

  public func pause() {
    onMain { [self] in
      wantsPlay = false
      player.pause()
    }
  }

  public func mute() {
    onMain { [self] in player.isMuted = true }
  }

  public func unmute() {
    onMain { [self] in player.isMuted = false }
  }

  public func retain(level: RetainLevel) {
    onMain { [self] in
      switch level {
      case .hot: item?.preferredForwardBufferDuration = 0
      case .warm: item?.preferredForwardBufferDuration = Self.warmBuffer
      case .cold: item?.preferredForwardBufferDuration = Self.coldBuffer
      }
    }
  }

  public func sample() -> SlotSample {
    sampleLock.withLock { cachedSample }
  }

  public func seek(positionMs: UInt64) {
    onMain { [self] in
      player.seek(to: CMTime(value: CMTimeValue(positionMs), timescale: 1000), toleranceBefore: .zero, toleranceAfter: .zero)
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
      layer.frame = host.bounds
      refreshSample()
    }
  }

  public func detach() {
    onMain { [self] in
      layer.removeFromSuperlayer()
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

  private func observe(_ item: AVPlayerItem) {
    itemReady = false
    visualReadySent = false
    observations = [
      item.observe(\.status, options: [.initial]) { [weak self] item, _ in self?.statusChanged(item) },
      layer.observe(\.isReadyForDisplay, options: [.initial]) { [weak self] _, _ in self?.displayChanged() }
    ]
    let center = NotificationCenter.default
    notificationTokens = [
      center.addObserver(forName: .AVPlayerItemDidPlayToEndTime, object: item, queue: .main) { [weak self] _ in
        self?.itemEnded()
      },
      center.addObserver(forName: .AVPlayerItemPlaybackStalled, object: item, queue: .main) { [weak self] _ in
        guard let self else { return }
        let position = positionMs()
        events.with { $0.stalled(slot: self.id, positionMs: position) }
      }
    ]
    timeToken = player.addPeriodicTimeObserver(forInterval: Self.sampleInterval, queue: .main) { [weak self] _ in
      self?.refreshSample()
    }
  }

  private func itemEnded() {
    guard looping else {
      events.with { $0.ended(slot: id) }
      return
    }
    player.seek(to: .zero) { [weak self] _ in
      guard let self, self.wantsPlay else { return }
      self.player.play()
    }
  }

  private func statusChanged(_ item: AVPlayerItem) {
    guard item === self.item else { return }
    switch item.status {
    case .readyToPlay:
      itemReady = true
      refreshSample()
      events.with { $0.ready(slot: id) }
      prerollIfReady()
    case .failed:
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
    observations.forEach { $0.invalidate() }
    observations = []
    notificationTokens.forEach { NotificationCenter.default.removeObserver($0) }
    notificationTokens = []
    if let timeToken { player.removeTimeObserver(timeToken) }
    timeToken = nil
    item?.asset.cancelLoading()
    player.pause()
    player.removeAllItems()
    item = nil
    prerollRequested = false
    itemReady = false
    visualReadySent = false
    looping = false
    refreshSample()
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
    sampleLock.withLock { cachedSample = sample }
  }

  private static func httpStatus(of item: AVPlayerItem) -> Int32 {
    Int32(item.errorLog()?.events.last?.errorStatusCode ?? 0)
  }
}
