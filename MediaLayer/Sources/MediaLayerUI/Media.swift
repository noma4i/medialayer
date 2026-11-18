import MediaLayer
import Nuke
import Observation
import SwiftUI

public protocol MediaPlaces: AnyObject, Sendable {
  func imageStats() -> ImageCounters
  func cacheKey(url: String, headers: [String: String]) -> String
  func mountPlace(item: MediaItem) throws -> any PlaceHandle
  func mountVideoWindow() throws -> any WindowHandle
  func mountImageWindow(name: String, rowsPerPage: Int, reversed: Bool) throws -> any WindowHandle
  func seek(key: String, fraction: Double) throws
  func setMutedAll(muted: Bool) throws
  func fetchImage(url: String, headers: [String: String]) async throws -> ImageFetchResult
  func placeholder(source: MediaItem, ready: Bool) async -> Placeholder
  func setTonePreparation(preparation: TonePreparation) throws
  func tonePixelsNeeded(source: MediaItem) async -> Bool
  func playUrl(source: MediaItem) async throws -> MediaPlayer
  func readRoute(item: MediaItem) async throws -> PlaybackRoute
  func retry(item: MediaItem) async throws
  func stateStream(key: String) -> AsyncStream<MediaState>
  func toneStream(source: MediaItem, band: Float) -> AsyncStream<ToneUpdate>
}

public protocol PlaceHandle: AnyObject, Sendable {
  func subscribePlayback(listener: any PlaybackListener) throws -> Subscription
  func update(input: PlaceInput) throws
  func seek(fraction: Double) throws
  func itemState(state: MediaState) throws
  func release()
}

public protocol WindowHandle: AnyObject, Sendable {
  func replace(active: UInt32, sources: [MediaItem?]) throws
  func replaceRows(rows: [[MediaItem]]) throws
  func showRows(first: UInt32, last: UInt32) throws
  func subscribeReady(listener: any WindowReadyListener) throws
  func moveTo(active: UInt32) throws
  func setActive(isActive: Bool) throws
  func release()
}

extension MediaPlace: PlaceHandle {}
extension MediaWindow: WindowHandle {}

extension MediaLayer: MediaPlaces {
  public func mountPlace(item: MediaItem) throws -> any PlaceHandle { try openPlace(item: item) }
  public func mountVideoWindow() throws -> any WindowHandle { try openVideoWindow() }
  public func mountImageWindow(name: String, rowsPerPage: Int, reversed: Bool) throws -> any WindowHandle {
    try openImageWindow(name: name, rowsPerPage: UInt32(rowsPerPage), reversed: reversed)
  }
}

@Observable
final class MutedFlag: @unchecked Sendable {
  var value = false
  var failure: MediaError? { didSet { TestChange.fire() } }
}

public enum Media {
  private static let flag = MutedFlag()
  nonisolated(unsafe) static weak var current: MediaPlaces?
  nonisolated(unsafe) public internal(set) static var audio: MediaAudioSession?
  nonisolated(unsafe) static var link: AudioLink?

  public static var muted: Bool { flag.value }

  public static func retry(_ source: some MediaSource) async throws {
    guard let places = ProcessMediaContext.shared.current?.places else { throw MediaError.Closed }
    try await places.retry(item: normalizeItem(item: source.item))
  }

  /// Applies the flag to the open layer first: a refusal of the core throws and leaves the flag unchanged.
  public static func setMuted(_ value: Bool) throws {
    do {
      try current?.setMutedAll(muted: value)
    } catch {
      flag.failure = mediaError(error)
      let native = error as NSError
      NativeFailures.publish(NativeFailure(source: .device, domain: native.domain, code: Int64(native.code), message: native.localizedDescription))
      throw error
    }
    flag.failure = nil
    NativeFailures.clear()
    flag.value = value
    ScreenAwake.shared.set(muted: value)
  }

  /// The last refusal of the core to apply the global mute, or nil after a success.
  public static var failure: MediaError? {
    if let error = NativeFailures.lastError { return error }
    if let failure = NativeFailures.last, failure.source == .file { return .Unexpected(detail: failure.message) }
    return flag.failure
  }

  public static var failures: AsyncStream<NativeFailure> { NativeFailures.events }

  static func publishFailure(_ failure: NativeFailure) {
    NativeFailures.publish(failure)
    flag.failure = .Unexpected(detail: failure.message)
  }

  static func recordNativeFailure(_ error: Error, source: FailureSource) {
    let native = error as NSError
    publishFailure(NativeFailure(source: source, domain: native.domain, code: Int64(native.code), message: native.localizedDescription))
  }

  static func recordFailure(_ error: Error) {
    let failure = mediaError(error)
    NativeFailures.record(failure)
    flag.failure = failure
  }

  static func clearFailure() {
    flag.failure = nil
    NativeFailures.clear()
  }
}

@Observable
final class ProcessMediaContext: @unchecked Sendable {
  static let shared = ProcessMediaContext()
  private(set) var current: MediaContext?

  func install(_ context: MediaContext) {
    current = context
  }

}

@propertyWrapper
struct ResolvedMediaContext: DynamicProperty {
  @Environment(\.mediaContext) private var environment

  var wrappedValue: MediaContext? { environment ?? ProcessMediaContext.shared.current }
}

public final class MediaContext: @unchecked Sendable {
  public let places: MediaPlaces
  public let slots: LoadedSlots
  let stats: ImageCounters
  var pipeline: Nuke.ImagePipeline { ImageMemory.pipeline(for: places) }

  public init(places: MediaPlaces, slots: LoadedSlots? = nil) {
    self.places = places
    self.slots = slots ?? LoadedSlots.of(places)
    stats = places.imageStats()
  }
}

private struct MediaContextKey: EnvironmentKey {
  static let defaultValue: MediaContext? = nil
}

extension EnvironmentValues {
  var mediaContext: MediaContext? {
    get { self[MediaContextKey.self] }
    set { self[MediaContextKey.self] = newValue }
  }
}

extension EnvironmentValues {
  public var mediaLayer: MediaLayer? {
    (mediaContext ?? ProcessMediaContext.shared.current)?.places as? MediaLayer
  }
}

final class OwnedMediaContext {
  private let media: MediaPlaces
  private let slots: LoadedSlots?
  private(set) lazy var context = MediaContext(places: media, slots: slots)

  init(media: MediaPlaces, slots: LoadedSlots?) {
    self.media = media
    self.slots = slots
  }
}

private struct MediaLayerModifier: ViewModifier {
  @State private var owned: OwnedMediaContext

  init(media: MediaPlaces, slots: LoadedSlots?) {
    _owned = State(initialValue: OwnedMediaContext(media: media, slots: slots))
  }

  func body(content: Content) -> some View {
    let context = owned.context
    return content
      .environment(\.mediaContext, context)
      .onAppear { ProcessMediaContext.shared.install(context) }
  }
}

private struct OpeningMediaLayerModifier: ViewModifier {
  let opening: Task<MediaLayer, Error>
  @State private var context: MediaContext?

  func body(content: Content) -> some View {
    content
      .environment(\.mediaContext, context)
      .task(id: opening) {
        do {
          let layer = try await opening.value
          guard !Task.isCancelled else { return }
          let ready = MediaContext(places: layer)
          context = ready
          ProcessMediaContext.shared.install(ready)
        } catch {
          guard !Task.isCancelled else { return }
          Media.recordFailure(error)
        }
      }
  }
}

extension View {
  public func mediaLayer(_ media: MediaPlaces, slots: LoadedSlots? = nil) -> some View {
    modifier(MediaLayerModifier(media: media, slots: slots)).id(ObjectIdentifier(media))
  }

  /// Draws the content at once. Media views declare no place until the task returns the layer; a failure of the task shows in `Media.failure`.
  public func mediaLayer(opening: Task<MediaLayer, Error>) -> some View {
    modifier(OpeningMediaLayerModifier(opening: opening))
  }
}
