import Foundation
import UniformTypeIdentifiers

public struct ImportProgress: Sendable, Equatable {
  public let bytesCopied: Int64
  public let totalBytes: Int64?

  public init(bytesCopied: Int64, totalBytes: Int64?) {
    self.bytesCopied = bytesCopied
    self.totalBytes = totalBytes
  }
}

extension MediaLayer {
  /// Progress runs on the copy thread. Hop to the main thread to update UI.
  public func importMedia(_ source: URL, directory: URL? = nil, progress: (@Sendable (ImportProgress) -> Void)? = nil) async throws -> PreparedMedia {
    let work = Task.detached { try importMediaFile(source, directory: directory ?? FileManager.default.temporaryDirectory, progress: progress) }
    return try await withTaskCancellationHandler { try await work.value } onCancel: { work.cancel() }
  }
}

func importMediaFile(
  _ source: URL,
  directory: URL = FileManager.default.temporaryDirectory,
  progress: ((ImportProgress) -> Void)? = nil,
  read: (FileHandle, Int) throws -> Data? = { try $0.read(upToCount: $1) },
  write: (FileHandle, Data) throws -> Void = { try $0.write(contentsOf: $1) }
) throws -> PreparedMedia {
  let scoped = source.startAccessingSecurityScopedResource()
  defer { if scoped { source.stopAccessingSecurityScopedResource() } }
  var output: URL?
  do {
    try Task.checkCancellation()
    let values = try source.resourceValues(forKeys: [.nameKey, .contentTypeKey, .fileSizeKey])
    let totalBytes = values.fileSize.map(Int64.init)
    let suffix = importExtension(name: values.name, mime: values.contentType?.preferredMIMEType)
    let name = UUID().uuidString + (suffix.isEmpty ? "" : ".\(suffix)")
    let destination = directory.appendingPathComponent(name)
    output = destination
    let input = try FileHandle(forReadingFrom: source)
    defer { try? input.close() }
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    guard FileManager.default.createFile(atPath: destination.path, contents: nil) else {
      throw MediaError.Fetch(detail: "the destination cannot be created")
    }
    let writer = try FileHandle(forWritingTo: destination)
    defer { try? writer.close() }
    var copied: Int64 = 0
    try Task.checkCancellation()
    progress?(ImportProgress(bytesCopied: copied, totalBytes: totalBytes))
    while true {
      try Task.checkCancellation()
      let bytes = try read(input, 64 * 1024) ?? Data()
      try Task.checkCancellation()
      if bytes.isEmpty { break }
      try write(writer, bytes)
      try Task.checkCancellation()
      copied += Int64(bytes.count)
      progress?(ImportProgress(bytesCopied: copied, totalBytes: totalBytes))
    }
    try writer.close()
    try Task.checkCancellation()
    return PreparedMedia(url: destination, isNew: true)
  } catch {
    if let output { NativeFailures.removeFile(output) }
    if error is CancellationError { throw error }
    throw error as? MediaError ?? MediaError.Fetch(detail: "the source cannot be imported: \(error.localizedDescription)")
  }
}
