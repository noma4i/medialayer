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
    let audio = MediaAudioSession()
    let work = Task.detached(priority: .userInitiated) {
      let (config, factory) = try makeConfig(
        defaults: try defaults(), fetcher: fetcher, settings: settings, directory: directory, silent: onSimulator, audio: audio, audioFocus: audioFocus, imageCache: imageCache
      )
      let resources = config.memoryReleaser as? OpenedResources
      do {
        try Task.checkCancellation()
        let media = try build(config)
        let link = AudioLink(core: media, ports: .system(focus: audioFocus.core))
        audio.bind(link)
        link.setVolumeMultiplier = { value in factory.slots.setVolumeMultiplier(value) }
        resources?.bind(media, link: link)
        #if os(iOS)
        CaptureFiles.sweepOrphans()
        #endif
        if Task.isCancelled {
          media.close()
          throw CancellationError()
        }
        return (config, factory, media, link)
      } catch {
        await resources?.shutdown()
        throw error
      }
    }
    let (_, factory, media, link) = try await withTaskCancellationHandler { try await work.value } onCancel: { work.cancel() }
    if Task.isCancelled {
      media.close()
      throw CancellationError()
    }
    try media.setTonePreparation(preparation: ToneFrames(places: media))
    ImageMemory.install(imageCache, on: media)
    LoadedSlots.install(factory.slots, on: media)
    HostLayers.install(factory.hosts, on: media)
    media.followSystem(events: media.playerEvents(), link: factory.events, center: .default, names: .system, audio: link)
    Media.current = media
    Media.audio = audio
    Media.link = link
    return media
  }

  static func makeConfig(
    defaults: SpecDefaults,
    fetcher: Fetcher,
    settings: SettingsStore,
    directory: URL?,
    silent: Bool,
    audio: MediaAudioSession? = nil,
    audioFocus: MediaAudioFocus = .mix,
    imageCache: ImageCache = ImageCache(costLimit: MediaImagePipeline.defaultMemoryLimit)
  ) throws -> (OpenConfig, AVQueuePlayerSlotFactory) {
    let factory = AVQueuePlayerSlotFactory(startMuted: silent, audio: audio)
    let directory = try directory ?? defaultDirectory()
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    let config = OpenConfig(
      dir: directory.path,
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
      silent: silent,
      audioFocus: audioFocus.core
    )
    return (config, factory)
  }

  fileprivate func stopFollowingSystem() {
    objc_setAssociatedObject(self, &lifetimeKey, nil, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
  }

  public var microphoneAvailability: AsyncStream<MicrophoneAvailability> {
    (objc_getAssociatedObject(self, &lifetimeKey) as? SystemSignals)?.audio?.availability ?? AsyncStream { $0.finish() }
  }

  public func callAudio(_ kind: MediaCallKind) async -> MediaCallAudio {
    guard let audio = (objc_getAssociatedObject(self, &lifetimeKey) as? SystemSignals)?.audio else {
      preconditionFailure("The media layer is closed")
    }
    return await audio.call(kind)
  }

  func followSystem(events: PlayerEventsProtocol, link: PlayerEventsLink, center: NotificationCenter, names: SystemSignalNames, audio: AudioLink? = nil) {
    link.bind(events)
    let signals = SystemSignals(events: events, center: center, names: names, audio: audio, onRelease: { link.bind(nil) })
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
  private var link: AudioLink?
  private var isClosed = false

  init(images: ImageMemoryReleaser, factory: AVQueuePlayerSlotFactory, audio: MediaAudioSession?) {
    self.images = images
    self.factory = factory
    self.audio = audio
  }

  func bind(_ media: MediaLayer, link: AudioLink) {
    lock.withLock { self.media = media; self.link = link }
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
    let link = lock.withLock { let link = self.link; self.link = nil; return link }
    link?.close()
    audio?.bind(nil)
    factory.release()
    images.shutdown()
    if let media, Media.current === media { Media.current = nil }
    if let audio, Media.audio === audio { Media.audio = nil }
    if let link, Media.link === link { Media.link = nil }
  }
}
