import Foundation

/// A source declared by value, without a class of its own. The core cleans the text once: a blank url, cover or blur hash is absent.
/// A source without a url is still created, its place fails with `MediaState.failed`.
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
    let clean = normalizeItem(item: MediaItem(kind: kind, url: url, headers: headers, cover: cover, blurHash: blurHash, strategy: strategy))
    self.url = clean.url
    self.kind = kind
    self.headers = headers
    self.cover = clean.cover
    self.blurHash = clean.blurHash
    self.size = size
    self.strategy = strategy
  }

  /// A local file by absolute path or `file://` text.
  public static func file(
    _ path: String,
    kind: Kind,
    cover: String? = nil,
    blurHash: String? = nil,
    size: MediaSize? = nil,
    strategy: CacheStrategy = .window
  ) -> MediaSourceValue {
    MediaSourceValue(url: localFileUrl(path: path), kind: kind, cover: cover, blurHash: blurHash, size: size, strategy: strategy)
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

  /// An `http` or `https` url.
  public static func remote(
    _ url: String,
    kind: Kind,
    headers: [String: String] = [:],
    cover: String? = nil,
    blurHash: String? = nil,
    size: MediaSize? = nil,
    strategy: CacheStrategy = .window
  ) -> MediaSourceValue {
    MediaSourceValue(url: remoteUrl(text: url), kind: kind, headers: headers, cover: cover, blurHash: blurHash, size: size, strategy: strategy)
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

  /// A video or an audio resource of a bundle. An image is a programming error, the text comes from the core. A missing resource gives a source without a url.
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
    if let refusal = bundledSourceError(kind: kind) { preconditionFailure(refusal) }
    let located = bundle.url(forResource: name, withExtension: fileExtension, subdirectory: subdirectory)
    return file(located?.absoluteString ?? "", kind: kind, cover: cover, blurHash: blurHash, size: size, strategy: strategy)
  }
}
