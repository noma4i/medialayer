import Foundation
import MediaLayer
import Nuke
import ObjectiveC

nonisolated(unsafe) private var lifetimeKey: UInt8 = 0

extension MediaLayer {
  public static func open(
    fetcher: Fetcher = URLSessionFetcher(),
    settings: SettingsStore = UserDefaultsSettings(),
    directory: URL? = nil,
    audioFocus: MediaAudioFocus = .mix,
    imageMemoryBytes: Int = MediaImagePipeline.defaultMemoryLimit
  ) async throws -> MediaLayer {
    try await open(
      defaults: { SpecDefaults(defaults: defaultLimits(), pool: defaultPoolSettings(), namespaceHeaders: []) },
      fetcher: fetcher, settings: settings, directory: directory, audioFocus: audioFocus, imageMemoryBytes: imageMemoryBytes
    )
  }

  public static func open(
    spec: String,
    fetcher: Fetcher = URLSessionFetcher(),
    settings: SettingsStore = UserDefaultsSettings(),
    directory: URL? = nil,
    audioFocus: MediaAudioFocus = .mix,
    imageMemoryBytes: Int = MediaImagePipeline.defaultMemoryLimit
  ) async throws -> MediaLayer {
    try await open(
      defaults: { try specDefaults(json: spec) }, fetcher: fetcher, settings: settings, directory: directory,
      audioFocus: audioFocus, imageMemoryBytes: imageMemoryBytes
    )
  }

  static var runsOnSimulator: Bool {
    #if targetEnvironment(simulator)
    true
    #else
    false
    #endif
  }

  static func open(
    defaults: @escaping @Sendable () throws -> SpecDefaults,
    fetcher: Fetcher,
    settings: SettingsStore,
    directory: URL?,
    audioFocus: MediaAudioFocus,
    imageMemoryBytes: Int,
    onSimulator: Bool = runsOnSimulator,
    build: @escaping @Sendable (OpenConfig) throws -> MediaLayer = { try MediaLayer.open(config: $0) }
  ) async throws -> MediaLayer {
    guard imageMemoryBytes > 0 else { throw MediaError.InvalidSpec(detail: "imageMemoryBytes must be positive") }
    let imageCache = ImageCache(costLimit: imageMemoryBytes)
    let audio = MediaAudioSession(focus: audioFocus)
    let work = Task.detached(priority: .userInitiated) {
      let (config, factory) = try makeConfig(
        defaults: try defaults(), fetcher: fetcher, settings: settings, directory: directory, silent: onSimulator, audio: audio, imageCache: imageCache
      )
      let resources = config.memoryReleaser as? OpenedResources
      do {
        try Task.checkCancellation()
        let media = try build(config)
        resources?.bind(media)
        if Task.isCancelled {
          media.close()
          throw CancellationError()
        }
        return (config, factory, media)
      } catch {
        await resources?.shutdown()
        throw error
      }
    }
    let (_, factory, media) = try await withTaskCancellationHandler { try await work.value } onCancel: { work.cancel() }
    if Task.isCancelled {
      media.close()
      throw CancellationError()
    }
    try media.setTonePreparation(preparation: ToneFrames(places: media))
    ImageMemory.install(imageCache, on: media)
    LoadedSlots.install(factory.slots, on: media)
    HostLayers.install(factory.hosts, on: media)
    media.followSystem(events: media.playerEvents(), link: factory.events, center: .default, names: .system)
    Media.current = media
    Media.audio = audio
    return media
  }

  static func makeConfig(
    defaults: SpecDefaults,
    fetcher: Fetcher,
    settings: SettingsStore,
    directory: URL?,
    silent: Bool,
    audio: MediaAudioSession? = nil,
    imageCache: ImageCache = ImageCache(costLimit: MediaImagePipeline.defaultMemoryLimit)
  ) throws -> (OpenConfig, AVQueuePlayerSlotFactory) {
    let factory = AVQueuePlayerSlotFactory(startMuted: silent, audio: audio)
    let directory = try directory ?? defaultDirectory()
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    let volume = try directory.resourceValues(forKeys: [.volumeAvailableCapacityForOpportunisticUsageKey])
    guard let available = volume.volumeAvailableCapacityForOpportunisticUsage else { throw CocoaError(.fileReadUnknown) }
    let config = OpenConfig(
      dir: directory.path,
      availableBytes: UInt64(clamping: available),
      fetcher: fetcher,
      settings: settings,
      slots: factory,
      device: deviceMemory(),
      defaults: defaults.defaults,
      pool: defaults.pool,
      namespaceHeaders: defaults.namespaceHeaders,
      imageWindows: defaults.imageWindows,
      forwardHeaders: defaults.forwardHeaders,
      evictVideo: defaults.evictVideo,
      evictImage: defaults.evictImage,
      evictAudio: defaults.evictAudio,
      memoryReleaser: OpenedResources(images: ImageMemoryReleaser(cache: imageCache), factory: factory, audio: audio),
      silent: silent
    )
    return (config, factory)
  }

  fileprivate func stopFollowingSystem() {
    objc_setAssociatedObject(self, &lifetimeKey, nil, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
  }

  func followSystem(events: PlayerEventsProtocol, link: PlayerEventsLink, center: NotificationCenter, names: SystemSignalNames) {
    link.bind(events)
    let signals = SystemSignals(events: events, center: center, names: names) { link.bind(nil) }
    objc_setAssociatedObject(self, &lifetimeKey, signals, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
  }

  static func deviceMemory() -> DeviceMemory {
    DeviceMemory(totalMb: UInt32(clamping: ProcessInfo.processInfo.physicalMemory / 1_048_576), heapClassMb: nil, lowRam: false)
  }

  private static func defaultDirectory() throws -> URL {
    let caches = try FileManager.default.url(for: .cachesDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
    let directory = caches.appendingPathComponent("MediaLayer", isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    return directory
  }
}

final class OpenedResources: MemoryReleaser, @unchecked Sendable {
  private let images: ImageMemoryReleaser
  private let factory: AVQueuePlayerSlotFactory
  private let audio: MediaAudioSession?
  private let lock = NSLock()
  private weak var media: MediaLayer?
  private var isClosed = false

  init(images: ImageMemoryReleaser, factory: AVQueuePlayerSlotFactory, audio: MediaAudioSession?) {
    self.images = images
    self.factory = factory
    self.audio = audio
  }

  func bind(_ media: MediaLayer) {
    lock.withLock { self.media = media }
  }

  func dropMemoryCaches() {
    images.dropMemoryCaches()
  }

  func shutdown() async {
    let media: MediaLayer?? = lock.withLock {
      guard !isClosed else { return nil }
      isClosed = true
      return .some(self.media)
    }
    guard let media else { return }
    media?.stopFollowingSystem()
    factory.release()
    images.shutdown()
    if let media, Media.current === media { Media.current = nil }
    if let audio, Media.audio === audio { Media.audio = nil }
  }
}
