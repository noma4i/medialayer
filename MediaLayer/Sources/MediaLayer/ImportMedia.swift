import Foundation
import UniformTypeIdentifiers

extension MediaLayer {
  public func importMedia(_ source: URL, kind: Kind) async throws -> PreparedMedia {
    let work = Task.detached { try importMediaFile(source) }
    return try await withTaskCancellationHandler { try await work.value } onCancel: { work.cancel() }
  }
}

func importMediaFile(
  _ source: URL,
  directory: URL = FileManager.default.temporaryDirectory,
  read: (FileHandle, Int) throws -> Data? = { try $0.read(upToCount: $1) },
  write: (FileHandle, Data) throws -> Void = { try $0.write(contentsOf: $1) }
) throws -> PreparedMedia {
  let scoped = source.startAccessingSecurityScopedResource()
  defer { if scoped { source.stopAccessingSecurityScopedResource() } }
  var output: URL?
  do {
    try Task.checkCancellation()
    let values = try source.resourceValues(forKeys: [.nameKey, .contentTypeKey])
    let suffix = importExtension(name: values.name, mime: values.contentType?.preferredMIMEType)
    let name = UUID().uuidString + (suffix.isEmpty ? "" : ".\(suffix)")
    let destination = directory.appendingPathComponent(name)
    output = destination
    let input = try FileHandle(forReadingFrom: source)
    defer { try? input.close() }
    guard FileManager.default.createFile(atPath: destination.path, contents: nil) else {
      throw MediaError.Fetch(detail: "the destination cannot be created")
    }
    let writer = try FileHandle(forWritingTo: destination)
    defer { try? writer.close() }
    while true {
      try Task.checkCancellation()
      let bytes = try read(input, 64 * 1024) ?? Data()
      try Task.checkCancellation()
      if bytes.isEmpty { break }
      try write(writer, bytes)
    }
    try writer.close()
    try Task.checkCancellation()
    return PreparedMedia(url: destination, isNew: true)
  } catch {
    if let output { try? FileManager.default.removeItem(at: output) }
    if error is CancellationError { throw error }
    throw error as? MediaError ?? MediaError.Fetch(detail: "the source cannot be imported: \(error.localizedDescription)")
  }
}
