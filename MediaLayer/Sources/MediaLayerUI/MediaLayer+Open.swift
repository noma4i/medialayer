import Foundation
import MediaLayer
import Nuke
import ObjectiveC

nonisolated(unsafe) private var lifetimeKey: UInt8 = 0

extension MediaLayer {
  public static func open(
    fetcher: Fetcher,
    settings: SettingsStore,
    directory: URL? = nil,
    startMuted: Bool = false,
    silent: Bool = false,
    imageMemoryBytes: Int = MediaImagePipeline.defaultMemoryLimit
  ) throws -> MediaLayer {
    let defaults = SpecDefaults(defaults: defaultLimits(), pool: defaultPoolSettings(), namespaceHeaders: [])
    return try open(
      defaults: defaults, fetcher: fetcher, settings: settings, directory: directory, startMuted: startMuted, silent: silent, imageMemoryBytes: imageMemoryBytes
    )
  }

  public static func open(
    spec: String,
    fetcher: Fetcher,
    settings: SettingsStore,
    directory: URL? = nil,
    startMuted: Bool = false,
    silent: Bool = false,
    imageMemoryBytes: Int = MediaImagePipeline.defaultMemoryLimit
  ) throws -> MediaLayer {
    try open(
      defaults: specDefaults(json: spec), fetcher: fetcher, settings: settings, directory: directory, startMuted: startMuted, silent: silent,
      imageMemoryBytes: imageMemoryBytes
    )
  }

  private static func open(
    defaults: SpecDefaults,
    fetcher: Fetcher,
    settings: SettingsStore,
    directory: URL?,
    startMuted: Bool,
    silent: Bool,
    imageMemoryBytes: Int
  ) throws -> MediaLayer {
    guard imageMemoryBytes > 0 else { throw MediaError.InvalidSpec(detail: "imageMemoryBytes must be positive") }
    let imageCache = ImageCache(costLimit: imageMemoryBytes)
    let (config, factory) = try makeConfig(
      defaults: defaults, fetcher: fetcher, settings: settings, directory: directory, startMuted: startMuted, silent: silent, imageCache: imageCache
    )
    let media = try MediaLayer.open(config: config)
    ImageMemory.install(imageCache, on: media)
    media.followSystem(events: media.playerEvents(), link: factory.events, center: .default, names: .system)
    Media.current = media
    return media
  }

  static func makeConfig(
    defaults: SpecDefaults,
    fetcher: Fetcher,
    settings: SettingsStore,
    directory: URL?,
    startMuted: Bool,
    silent: Bool,
    imageCache: ImageCache = ImageCache(costLimit: MediaImagePipeline.defaultMemoryLimit)
  ) throws -> (OpenConfig, AVQueuePlayerSlotFactory) {
    let factory = AVQueuePlayerSlotFactory(startMuted: startMuted || silent || isDebugBuild)
    let config = OpenConfig(
      dir: try (directory ?? defaultDirectory()).path,
      fetcher: fetcher,
      settings: settings,
      slots: factory,
      device: deviceMemory(),
      defaults: defaults.defaults,
      pool: defaults.pool,
      namespaceHeaders: defaults.namespaceHeaders,
      imageWindows: defaults.imageWindows,
      memoryReleaser: ImageMemoryReleaser(cache: imageCache),
      silent: silent
    )
    return (config, factory)
  }

  func followSystem(events: PlayerEventsProtocol, link: PlayerEventsLink, center: NotificationCenter, names: SystemSignalNames) {
    link.bind(events)
    let signals = SystemSignals(events: events, center: center, names: names) { link.bind(nil) }
    objc_setAssociatedObject(self, &lifetimeKey, signals, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
  }

  static func deviceMemory() -> DeviceMemory {
    DeviceMemory(totalMb: UInt32(clamping: ProcessInfo.processInfo.physicalMemory / 1_048_576), heapClassMb: nil, lowRam: false)
  }

  private static var isDebugBuild: Bool {
    #if DEBUG
    true
    #else
    false
    #endif
  }

  private static func defaultDirectory() throws -> URL {
    let caches = try FileManager.default.url(for: .cachesDirectory, in: .userDomainMask, appropriateFor: nil, create: true)
    let directory = caches.appendingPathComponent("MediaLayer", isDirectory: true)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    return directory
  }
}
