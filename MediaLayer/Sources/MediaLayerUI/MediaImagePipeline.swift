import Foundation
import MediaLayer
import Nuke
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
        if data.source == .cache { places.imageStats().record(work: .diskHit) }
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
    configuration.imageCache = ImageMemory.cache(for: places)
    return ImagePipeline(configuration: configuration, delegate: ImageEvents(stats: places.imageStats()))
  }

  public static func decodeSize(for source: some MediaSource, points: CGSize, scale: CGFloat, contentMode: ContentMode) -> CGSize? {
    let declared = source.size.map { PixelSize(width: UInt32(clamping: $0.width), height: UInt32(clamping: $0.height)) }
    return imagePixelSize(width: Double(points.width), height: Double(points.height), scale: Double(scale), declared: declared, crop: contentMode == .fill)
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
    let pixels = decodeSize(for: source, points: points, scale: scale, contentMode: contentMode)
    let identity = places.cacheKey(url: url ?? source.url, headers: source.headers)
    let key = ImageKey(identity: identity, pixels: pixels, contentMode: contentMode)
    guard let address = URL(string: url ?? source.url) else { return (key, nil) }
    var urlRequest = URLRequest(url: address)
    for (name, value) in source.headers { urlRequest.setValue(value, forHTTPHeaderField: name) }
    let processors: [any ImageProcessing] = pixels.map {
      [ImageProcessors.Resize(size: $0, unit: .pixels, contentMode: contentMode == .fill ? .aspectFill : .aspectFit)]
    } ?? []
    var request = ImageRequest(urlRequest: urlRequest, processors: processors)
    request.userInfo["placementPixels"] = UInt64(max(0, points.width * scale) * max(0, points.height * scale))
    request.thumbnail = pixels.map { ImageRequest.ThumbnailOptions(size: $0, unit: .pixels, contentMode: contentMode == .fill ? .aspectFill : .aspectFit) }
    request.imageID = identity
    return (key, request)
  }
}

extension MediaImagePipeline {
  static func cachedImage(for request: ImageRequest, in pipeline: ImagePipeline) -> PlatformImage? {
    cachedContainer(for: request, in: pipeline)?.image
  }

  static func cachedContainer(for request: ImageRequest, in pipeline: ImagePipeline) -> ImageContainer? {
    pipeline.cache[request]
  }

  static func load(_ source: some MediaSource, request: ImageRequest, places: MediaPlaces, pipeline: ImagePipeline) async -> ImageContainer? {
    guard !Task.isCancelled else { return nil }
    places.itemState(key: source.url, state: .loading)
    do {
      let container = try await pipeline.imageTask(with: request).response.container
      places.itemState(key: source.url, state: .ready)
      return container
    } catch {
      if !Task.isCancelled { places.itemState(key: source.url, state: .failed(detail: error.localizedDescription)) }
      return nil
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
  }

  public func shutdown() {
    dropMemoryCaches()
  }
}
