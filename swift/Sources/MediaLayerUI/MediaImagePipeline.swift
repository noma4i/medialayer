import Foundation
import MediaLayer
import Nuke

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
  public static func make(places: MediaPlaces) -> ImagePipeline {
    var configuration = ImagePipeline.Configuration(dataLoader: CoreDataLoader(places: places))
    configuration.dataCache = nil
    configuration.imageCache = ImageCache.shared
    return ImagePipeline(configuration: configuration)
  }

  public static func request(for source: some MediaSource, url: String? = nil) -> ImageRequest? {
    guard let address = URL(string: url ?? source.url) else { return nil }
    var urlRequest = URLRequest(url: address)
    for (name, value) in source.headers { urlRequest.setValue(value, forHTTPHeaderField: name) }
    let processors: [any ImageProcessing] = source.size.map {
      [ImageProcessors.Resize(size: CGSize(width: $0.width, height: $0.height), unit: .pixels, contentMode: .aspectFill)]
    } ?? []
    return ImageRequest(urlRequest: urlRequest, processors: processors)
  }
}

extension MediaImagePipeline {
  static func load(_ source: some MediaSource, places: MediaPlaces, pipeline: ImagePipeline) async -> PlatformImage? {
    guard let request = request(for: source) else { return nil }
    places.itemState(key: source.url, state: .loading)
    do {
      let image = try await pipeline.image(for: request)
      places.itemState(key: source.url, state: .ready)
      return image
    } catch {
      places.itemState(key: source.url, state: .failed(detail: error.localizedDescription))
      return nil
    }
  }
}

public final class ImageMemoryReleaser: MemoryReleaser, @unchecked Sendable {
  public init() {}

  public func dropMemoryCaches() {
    ImageCache.shared.removeAll()
    let capacity = URLCache.shared.memoryCapacity
    URLCache.shared.memoryCapacity = 0
    URLCache.shared.memoryCapacity = capacity
  }
}
