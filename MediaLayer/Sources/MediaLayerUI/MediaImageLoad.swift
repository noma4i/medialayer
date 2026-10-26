import Foundation
import ImageIO
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

  /// The picture at `url` as an upright image with the long side cut to `side`. With `circle` the result is a square of `side` pixels cut to a circle.
  /// Works without an open layer, the decode takes its raster from the shared memory budget when a layer is open.
  public static func loadBitmap(
    url: URL,
    headers: [String: String] = [:],
    directory: URL,
    side: Int,
    circle: Bool = false,
    cacheBytes: UInt64 = defaultCacheBytes,
    timeout: TimeInterval = defaultTimeout,
    fetcher: Fetcher? = nil
  ) async throws -> PlatformImage {
    let fetched = try await load(url: url, headers: headers, directory: directory, cacheBytes: cacheBytes, timeout: timeout, fetcher: fetcher)
    let admit: DecodeAdmit
    if let layer = Media.current as? MediaLayer {
      admit = layer.decodeAdmit
    } else {
      admit = { _ in {} }
    }
    return try await bitmap(of: fetched.bytes, side: side, circle: circle, admit: admit)
  }

  static func bitmap(of bytes: Data, side: Int, circle: Bool, admit: @escaping DecodeAdmit) async throws -> PlatformImage {
    try await MediaImageDecode.decode(maxSide: side, circle: circle, admit: admit) { CGImageSourceCreateWithData(bytes as CFData, nil) }
  }
}
