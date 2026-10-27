import Foundation

/// A source declared by value, without a class of its own. The initializers keep the text as given and call nothing native.
/// The core cleans it once when the place is declared: a text that is no source url, a blank cover or blur hash is absent, and a place without a url fails with `MediaState.failed`.
public struct MediaSourceValue: MediaSource, Hashable {
  public let url: String
  public let kind: Kind
  public let headers: [String: String]
  public let cover: String?
  public let blurHash: String?
  public let size: MediaSize?
  public let strategy: CacheStrategy

  public init(
    url: String,
    kind: Kind,
    headers: [String: String] = [:],
    cover: String? = nil,
    blurHash: String? = nil,
    size: MediaSize? = nil,
    strategy: CacheStrategy = .window
  ) {
    self.url = url
    self.kind = kind
    self.headers = headers
    self.cover = cover
    self.blurHash = blurHash
    self.size = size
    self.strategy = strategy
  }

  /// A local file by absolute path or `file://` text, the core refuses any other text.
  public static func file(
    _ path: String,
    kind: Kind,
    cover: String? = nil,
    blurHash: String? = nil,
    size: MediaSize? = nil,
    strategy: CacheStrategy = .window
  ) -> MediaSourceValue {
    MediaSourceValue(url: path, kind: kind, cover: cover, blurHash: blurHash, size: size, strategy: strategy)
  }

  public static func file(
    _ url: URL,
    kind: Kind,
    cover: String? = nil,
    blurHash: String? = nil,
    size: MediaSize? = nil,
    strategy: CacheStrategy = .window
  ) -> MediaSourceValue {
    file(url.absoluteString, kind: kind, cover: cover, blurHash: blurHash, size: size, strategy: strategy)
  }

  /// An `http` or `https` url, the core refuses any other text.
  public static func remote(
    _ url: String,
    kind: Kind,
    headers: [String: String] = [:],
    cover: String? = nil,
    blurHash: String? = nil,
    size: MediaSize? = nil,
    strategy: CacheStrategy = .window
  ) -> MediaSourceValue {
    MediaSourceValue(url: url, kind: kind, headers: headers, cover: cover, blurHash: blurHash, size: size, strategy: strategy)
  }

  public static func remote(
    _ url: URL,
    kind: Kind,
    headers: [String: String] = [:],
    cover: String? = nil,
    blurHash: String? = nil,
    size: MediaSize? = nil,
    strategy: CacheStrategy = .window
  ) -> MediaSourceValue {
    remote(url.absoluteString, kind: kind, headers: headers, cover: cover, blurHash: blurHash, size: size, strategy: strategy)
  }

  /// A file resource of a bundle, a video, an audio or an image. `MediaImage` decodes an image file off the main thread. A missing resource gives a source without a url. An asset catalog name has no file, use `Image(_:)` for it.
  public static func bundled(
    _ name: String,
    extension fileExtension: String,
    subdirectory: String? = nil,
    bundle: Bundle = .main,
    kind: Kind,
    cover: String? = nil,
    blurHash: String? = nil,
    size: MediaSize? = nil,
    strategy: CacheStrategy = .window
  ) -> MediaSourceValue {
    let located = bundle.url(forResource: name, withExtension: fileExtension, subdirectory: subdirectory)
    return file(located?.absoluteString ?? "", kind: kind, cover: cover, blurHash: blurHash, size: size, strategy: strategy)
  }
}
