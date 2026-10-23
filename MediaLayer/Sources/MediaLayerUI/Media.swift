import MediaLayer
import Nuke
import Observation
import SwiftUI

public protocol MediaPlaces: AnyObject, Sendable {
  func imageStats() -> ImageCounters
  func cacheKey(url: String, headers: [String: String]) -> String
  func visible(source: MediaItem, visible: Bool)
  func setPlaying(key: String, playing: Bool, group: String?)
  func setPeeking(key: String, peeking: Bool)
  func setMuted(key: String, muted: Bool)
  func setRate(key: String, rate: Float)
  func setMutedAll(muted: Bool)
  func seek(key: String, fraction: Double)
  func itemState(key: String, state: MediaState)
  func fetchImage(url: String, headers: [String: String]) async throws -> ImageFetchResult
  func placeholder(source: MediaItem, ready: Bool) async -> Placeholder
  func setTonePreparation(preparation: TonePreparation)
  func tonePixelsNeeded(source: MediaItem) async -> Bool
  func playUrl(source: MediaItem) async throws -> PlaybackRoute
  func stateStream(key: String) -> AsyncStream<MediaState>
  func attachView(key: String, token: UInt64)
  func detachView(key: String)
}

extension MediaLayer: MediaPlaces {
  public func attachView(key: String, token: UInt64) {
    playerEvents().attach(key: key, viewToken: token)
  }

  public func detachView(key: String) {
    playerEvents().detach(key: key)
  }
}

@Observable
final class MutedFlag: @unchecked Sendable {
  var value = false
}

public enum Media {
  private static let flag = MutedFlag()
  nonisolated(unsafe) static weak var current: MediaPlaces?
  nonisolated(unsafe) public internal(set) static var audio: MediaAudioSession?

  public static var muted: Bool {
    get { flag.value }
    set {
      flag.value = newValue
      current?.setMutedAll(muted: newValue)
    }
  }
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
    places.setTonePreparation(preparation: ToneFrames(places: places))
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

private struct MediaLayerModifier: ViewModifier {
  @State private var context: MediaContext

  init(media: MediaPlaces, slots: LoadedSlots?) {
    _context = State(initialValue: MediaContext(places: media, slots: slots))
  }

  func body(content: Content) -> some View {
    content.environment(\.mediaContext, context)
  }
}

extension View {
  public func mediaLayer(_ media: MediaPlaces, slots: LoadedSlots? = nil) -> some View {
    modifier(MediaLayerModifier(media: media, slots: slots)).id(ObjectIdentifier(media))
  }
}
