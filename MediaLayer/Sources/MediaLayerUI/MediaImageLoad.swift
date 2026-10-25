import Foundation
import MediaLayer

public enum MediaImageLoad {
  public static let defaultCacheBytes: UInt64 = 8 << 20
  public static let defaultTimeout: TimeInterval = 5

  public static func load(
    url: URL,
    headers: [String: String] = [:],
    directory: URL,
    cacheBytes: UInt64 = defaultCacheBytes,
    timeout: TimeInterval = defaultTimeout,
    fetcher: Fetcher? = nil
  ) async throws -> ImageFetchResult {
    let request = ImageLoadRequest(
      url: url.absoluteString,
      headers: headers,
      dir: directory.path,
      cacheBytes: cacheBytes,
      timeoutMs: UInt64(timeout * 1000)
    )
    let source = fetcher ?? URLSessionFetcher(timeout: timeout)
    return try await withCheckedThrowingContinuation { continuation in
      DispatchQueue.global(qos: .utility).async {
        continuation.resume(with: Result { try loadImage(request: request, fetcher: source) })
      }
    }
  }
}
