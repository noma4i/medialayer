import Foundation
import MediaLayer
import Nuke
import SwiftUI

struct CoreDataLoader: DataLoading {
  let places: MediaPlaces

  func loadData(
    with request: URLRequest,
    didReceiveData: @escaping @Sendable (Data, URLResponse) -> Void,
    completion: @escaping @Sendable (Error?) -> Void
  ) -> any Nuke.Cancellable {
    guard let url = request.url else {
      completion(URLError(.badURL))
      return TaskCancellable(task: nil)
    }
    let places = places
    let headers = request.allHTTPHeaderFields ?? [:]
    let task = Task {
      do {
        let data = try await places.fetchImage(url: url.absoluteString, headers: headers)
        didReceiveData(data, HTTPURLResponse(url: url, statusCode: 200, httpVersion: nil, headerFields: nil) ?? URLResponse())
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
    return ImagePipeline(configuration: configuration)
  }

  public static func decodeSize(for source: some MediaSource, points: CGSize, scale: CGFloat) -> CGSize? {
    let declared = source.size.map { PixelSize(width: UInt32(clamping: $0.width), height: UInt32(clamping: $0.height)) }
    return imagePixelSize(width: Double(points.width), height: Double(points.height), scale: Double(scale), declared: declared)
      .map { CGSize(width: Int($0.width), height: Int($0.height)) }
  }

  public static func request(
    for source: some MediaSource,
    url: String? = nil,
    points: CGSize = .zero,
    scale: CGFloat = 1,
    contentMode: ContentMode = .fill
  ) -> ImageRequest? {
    guard let address = URL(string: url ?? source.url) else { return nil }
    var urlRequest = URLRequest(url: address)
    for (name, value) in source.headers { urlRequest.setValue(value, forHTTPHeaderField: name) }
    let processors: [any ImageProcessing] = decodeSize(for: source, points: points, scale: scale).map {
      [ImageProcessors.Resize(size: $0, unit: .pixels, contentMode: contentMode == .fill ? .aspectFill : .aspectFit)]
    } ?? []
    return ImageRequest(urlRequest: urlRequest, processors: processors)
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
      guard !Task.isCancelled else { return nil }
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
}
