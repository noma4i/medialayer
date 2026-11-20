import Foundation

/// The file to send. `isNew` is false when the source already fitted and is handed back untouched: the caller owns a file only when `isNew` is true.
public struct PreparedMedia: Equatable, Sendable {
  public let url: URL
  public let isNew: Bool

  public init(url: URL, isNew: Bool) {
    self.url = url
    self.isNew = isNew
  }
}

func preparedOutput(extension fileExtension: String, in directory: URL = FileManager.default.temporaryDirectory) -> URL {
  directory.appendingPathComponent("\(UUID().uuidString).\(fileExtension)")
}
