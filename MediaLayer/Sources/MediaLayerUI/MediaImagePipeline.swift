import Foundation
import ImageIO
import MediaLayer
@_spi(AsyncImageDecoding) import Nuke
import SwiftUI

struct CoreDataLoader: DataLoading {
  weak var places: MediaPlaces?

  func loadData(
    with request: URLRequest,
    didReceiveData: @escaping @Sendable (Data, URLResponse) -> Void,
    completion: @escaping @Sendable (Error?) -> Void
  ) -> any Nuke.Cancellable {
    guard let url = request.url else {
      completion(URLError(.badURL))
      return TaskCancellable(task: nil)
    }
    guard let places else {
      completion(CancellationError())
      return TaskCancellable(task: nil)
    }
    let headers = request.allHTTPHeaderFields ?? [:]
    let task = Task {
      do {
        let data = try await places.fetchImage(url: url.absoluteString, headers: headers)
        if data.source == .cache { places.imageStats().recordLater(work: .diskHit) }
        didReceiveData(data.bytes, HTTPURLResponse(url: url, statusCode: 200, httpVersion: nil, headerFields: nil) ?? URLResponse())
        completion(nil)
      } catch {
        completion(error)
      }
    }
    return TaskCancellable(task: task)
  }
}

private struct TaskCancellable: Nuke.Cancellable {
  let task: Task<Void, Never>?

  func cancel() { task?.cancel() }
}

public enum MediaImagePipeline {
  public static let defaultMemoryLimit = 128 * 1024 * 1024

  public static func make(places: MediaPlaces) -> ImagePipeline {
    var configuration = ImagePipeline.Configuration(dataLoader: CoreDataLoader(places: places))
    configuration.dataCache = nil
    configuration.isLocalResourcesSupportEnabled = false
    configuration.imageCache = ImageMemory.cache(for: places)
    let admit = FrameCache.admit(from: places as? MediaLayer)
    configuration.makeImageDecoder = { context in AnimatedKeepingDecoder(context: context, admit: admit) }
    return ImagePipeline(configuration: configuration, delegate: ImageEvents(stats: places.imageStats()))
  }

  public static func decodeSize(places: MediaPlaces, for source: some MediaSource, points: CGSize, scale: CGFloat, contentMode: ContentMode) -> CGSize? {
    let declared = source.size.map { PixelSize(width: UInt32(clamping: $0.width), height: UInt32(clamping: $0.height)) }
    return PixelSizeMemo.of(places).size(width: Double(points.width), height: Double(points.height), scale: Double(scale), declared: declared, crop: contentMode == .fill)
      .map { CGSize(width: Int($0.width), height: Int($0.height)) }
  }

  public static func request(
    places: MediaPlaces,
    for source: some MediaSource,
    url: String? = nil,
    points: CGSize = .zero,
    scale: CGFloat = 1,
    contentMode: ContentMode = .fill
  ) -> ImageRequest? {
    prepare(places: places, for: source, url: url, points: points, scale: scale, contentMode: contentMode).request
  }

  static func prepare(places: MediaPlaces, for source: some MediaSource, url: String? = nil, points: CGSize, scale: CGFloat, contentMode: ContentMode) -> (key: ImageKey, request: ImageRequest?) {
    let declared = source.size.map { PixelSize(width: UInt32(clamping: $0.width), height: UInt32(clamping: $0.height)) }
    let prepared = PixelSizeMemo.of(places).prepare(url: url ?? source.url, headers: source.headers, width: Double(points.width), height: Double(points.height), scale: Double(scale), declared: declared, crop: contentMode == .fill) { places.cacheKey(url: url ?? source.url, headers: source.headers) }
    return makeRequest(for: source, url: url, points: points, scale: scale, contentMode: contentMode, prepared: prepared)
  }

  static func cachedPreparation(places: MediaPlaces, for source: some MediaSource, points: CGSize, scale: CGFloat, contentMode: ContentMode) -> (key: ImageKey, request: ImageRequest?)? {
    let memo = PixelSizeMemo.of(places)
    let declared = source.size.map { PixelSize(width: UInt32(clamping: $0.width), height: UInt32(clamping: $0.height)) }
    guard let prepared = memo.cachedPreparation(url: source.url, headers: source.headers, width: Double(points.width), height: Double(points.height), scale: Double(scale), declared: declared, crop: contentMode == .fill) else { return nil }
    return makeRequest(for: source, points: points, scale: scale, contentMode: contentMode, prepared: prepared)
  }

  @concurrent
  static func prepareOffMain(places: MediaPlaces, for source: some MediaSource, points: CGSize, scale: CGFloat, contentMode: ContentMode = .fill) async -> (key: ImageKey, request: ImageRequest?) {
    prepare(places: places, for: source, points: points, scale: scale, contentMode: contentMode)
  }

  private static func makeRequest(for source: some MediaSource, url: String? = nil, points: CGSize, scale: CGFloat, contentMode: ContentMode, prepared: PixelSizeMemo.Prepared) -> (key: ImageKey, request: ImageRequest?) {
    let pixels = prepared.pixels.map { CGSize(width: Int($0.width), height: Int($0.height)) }
    let key = ImageKey(identity: prepared.identity, pixels: pixels, contentMode: contentMode)
    guard let address = URL(string: url ?? source.url) else { return (key, nil) }
    var urlRequest = URLRequest(url: address)
    for (name, value) in source.headers { urlRequest.setValue(value, forHTTPHeaderField: name) }
    let processors: [any ImageProcessing] = pixels.map {
      [ImageProcessors.Resize(size: $0, unit: .pixels, contentMode: contentMode == .fill ? .aspectFill : .aspectFit)]
    } ?? []
    var request = ImageRequest(urlRequest: urlRequest, processors: processors)
    request.userInfo["placementPixels"] = UInt64(max(0, points.width * scale) * max(0, points.height * scale))
    request.userInfo["decodePixels"] = pixels
    request.thumbnail = pixels.map { ImageRequest.ThumbnailOptions(size: $0, unit: .pixels, contentMode: contentMode == .fill ? .aspectFill : .aspectFit) }
    request.userInfo["imagePreparation"] = prepared
    request.imageID = prepared.identity
    return (key, request)
  }
}

extension MediaImagePipeline {
  static func interimImage(for source: some MediaSource, in context: MediaContext) -> PlatformImage? {
    guard let request = context.stillImages.request(url: source.url, headers: source.headers) else { return nil }
    return context.pipeline.cache[request]?.image
  }

  static func cachedImage(for request: ImageRequest, in pipeline: ImagePipeline) -> PlatformImage? {
    cachedContainer(for: request, in: pipeline)?.image
  }

  static func cachedContainer(for request: ImageRequest, in pipeline: ImagePipeline) -> ImageContainer? {
    pipeline.cache[request]
  }

  static func load(_ source: some MediaSource, request: ImageRequest, pipeline: ImagePipeline, index: StillImageIndex? = nil, report: (@Sendable (MediaState) -> Void)? = nil) async -> ImageContainer? {
    guard !Task.isCancelled else { return nil }
    report?(.loading)
    do {
      let container = try await pipeline.imageTask(with: request).response.container
      if container.data.map({ AnimatedFrames.sourceFrameCount($0) <= 1 }) ?? true {
        let pixels = request.userInfo["decodePixels"] as? CGSize ?? container.image.size
        index?.record(url: source.url, headers: source.headers, request: request, area: pixels.width * pixels.height)
      }
      report?(.ready)
      return container
    } catch {
      if !Task.isCancelled { report?(.failed(detail: error.localizedDescription)) }
      return nil
    }
  }
}

final class StillImageIndex: @unchecked Sendable {
  private struct Key: Hashable {
    let url: String
    let headers: [String: String]
  }
  private struct Entry {
    let request: ImageRequest
    let area: CGFloat
    var used: UInt64
  }
  private let lock = NSLock()
  private var entries: [Key: Entry] = [:]
  private var sequence: UInt64 = 0

  func request(url: String, headers: [String: String]) -> ImageRequest? {
    lock.withLock {
      let key = Key(url: url, headers: headers)
      guard var entry = entries[key] else { return nil }
      sequence += 1
      entry.used = sequence
      entries[key] = entry
      return entry.request
    }
  }

  func record(url: String, headers: [String: String], request: ImageRequest, area: CGFloat) {
    lock.withLock {
      let key = Key(url: url, headers: headers)
      sequence += 1
      if var entry = entries[key], entry.area > area {
        entry.used = sequence
        entries[key] = entry
        return
      }
      if entries[key] == nil, entries.count == 512, let oldest = entries.min(by: { $0.value.used < $1.value.used })?.key {
        entries.removeValue(forKey: oldest)
      }
      entries[key] = Entry(request: request, area: area, used: sequence)
    }
  }
}

public final class ImageMemoryReleaser: MemoryReleaser, @unchecked Sendable {
  private let cache: ImageCache
  private let frames: FrameCache

  public init(cache: ImageCache = .shared) {
    self.cache = cache
    frames = .shared
  }

  init(cache: ImageCache, frames: FrameCache) {
    self.cache = cache
    self.frames = frames
  }

  public func dropMemoryCaches() {
    cache.removeAll()
    frames.removeAll()
    let capacity = URLCache.shared.memoryCapacity
    URLCache.shared.memoryCapacity = 0
    URLCache.shared.memoryCapacity = capacity
    TestChange.fire()
  }

  public func shutdown() {
    dropMemoryCaches()
  }
}

/// The default decoder keeps the bytes of a GIF only; an animated WebP needs its bytes as well to be played frame by frame.
struct AnimatedKeepingDecoder: AsyncImageDecoding {
  private let base: ImageDecoders.Default
  private let pixels: CGSize?
  private let admit: FrameCache.Admit
  private let prepared: PixelSizeMemo.Prepared?

  init?(context: ImageDecodingContext, admit: @escaping FrameCache.Admit) {
    guard let base = ImageDecoders.Default(context: context) else { return nil }

    self.base = base
    pixels = context.request.userInfo["decodePixels"] as? CGSize
    prepared = context.request.userInfo["imagePreparation"] as? PixelSizeMemo.Prepared
    self.admit = admit
  }

  static func keepsSource(frames: Int) -> Bool { frames > 1 }

  static func decodeBytes(pixels: CGSize?, data: Data) -> UInt64? {
    if let pixels { return decodeCost(width: UInt32(clamping: Int(pixels.width)), height: UInt32(clamping: Int(pixels.height))) }
    guard let source = CGImageSourceCreateWithData(data as CFData, nil),
          let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
          let width = properties[kCGImagePropertyPixelWidth] as? Int,
          let height = properties[kCGImagePropertyPixelHeight] as? Int
    else { return nil }

    return decodeCost(width: UInt32(clamping: width), height: UInt32(clamping: height))
  }

  func decode(_ data: Data) async throws -> ImageContainer {
    guard let bytes = Self.decodeBytes(pixels: pixels, data: data) else {
      throw MediaError.Fetch(detail: "the image size cannot be read or is above the pixel limit")
    }
    guard let release = await admit(bytes) else { throw MediaError.Overloaded(detail: "the decode was not admitted") }
    defer { release() }

    var container = try base.decode(data)
    container.userInfo["imagePreparation"] = prepared
    if container.type == .webp, Self.keepsSource(frames: AnimatedFrames.sourceFrameCount(data)) { container.data = data }
    return container
  }

  func decodePartiallyDownloadedData(_ data: Data) -> ImageContainer? {
    base.decodePartiallyDownloadedData(data)
  }
}
