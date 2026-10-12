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
}

extension MediaSource {
  public var cover: String? { nil }
  public var blurHash: String? { nil }
  public var size: MediaSize? { nil }

  public var item: MediaItem {
    MediaItem(kind: kind, url: url, headers: headers, cover: cover, blurHash: blurHash)
  }
}

extension MediaLayer {
  public func replaceImageWindow<Source: MediaSource>(_ name: String, active: Int, pages: [[Source]]) {
    replaceImageWindow(name: name, active: UInt32(clamping: max(active, 0)), pages: pages.map { $0.map(\.item) })
  }
}
