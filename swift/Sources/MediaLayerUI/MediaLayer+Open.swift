import Foundation
import MediaLayer
import ObjectiveC

nonisolated(unsafe) private var lifetimeKey: UInt8 = 0

extension MediaLayer {
  public static func open(
    fetcher: Fetcher,
    settings: SettingsStore,
    directory: URL? = nil,
    startMuted: Bool = false
  ) throws -> MediaLayer {
    let factory = AVQueuePlayerSlotFactory(startMuted: startMuted || Self.isDebugBuild)
    let media = try MediaLayer.open(config: OpenConfig(
      dir: try (directory ?? Self.defaultDirectory()).path,
      fetcher: fetcher,
      settings: settings,
      slots: factory,
      device: Self.deviceMemory(),
      defaults: defaultLimits(),
      pool: defaultPoolSettings(),
      namespaceHeaders: [],
      memoryReleaser: ImageMemoryReleaser()
    ))
    media.followSystem(events: media.playerEvents(), link: factory.events, center: .default, names: .system)
    Media.current = media
    return media
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
