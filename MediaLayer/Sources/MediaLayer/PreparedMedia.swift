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

func preparableFile(_ url: String) throws -> URL {
  guard isLocalMediaUrl(url: url),
        let file = URL(string: url.trimmingCharacters(in: .whitespacesAndNewlines)), file.isFileURL
  else { throw MediaError.Fetch(detail: "\(url) is not a file:// source") }
  guard FileManager.default.isReadableFile(atPath: file.path) else { throw MediaError.Fetch(detail: "\(url) is not a readable file") }
  return file
}

func preparedOutput(extension fileExtension: String) -> URL {
  FileManager.default.temporaryDirectory.appendingPathComponent("\(UUID().uuidString).\(fileExtension)")
}
