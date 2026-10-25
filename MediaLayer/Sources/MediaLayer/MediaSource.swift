public struct MediaSize: Equatable, Sendable {
  public let width: Int
  public let height: Int

  public init(width: Int, height: Int) {
    self.width = width
    self.height = height
  }
}

public protocol MediaSource: Sendable {
  var url: String { get }
  var kind: Kind { get }
  var headers: [String: String] { get }
  var cover: String? { get }
  var blurHash: String? { get }
  var size: MediaSize? { get }
  var strategy: CacheStrategy { get }
}

extension MediaSource {
  public var cover: String? { nil }
  public var blurHash: String? { nil }
  public var size: MediaSize? { nil }
  public var strategy: CacheStrategy { .window }

  public var item: MediaItem {
    MediaItem(kind: kind, url: url, headers: headers, cover: cover, blurHash: blurHash, strategy: strategy)
  }
}
