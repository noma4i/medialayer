import AVFoundation
import Foundation

extension MediaLayer {
  /// Length and frame of a local video or audio file, read without a player. A remote source has no metadata without a download and gives `nil`.
  /// A file that is not media gives `nil`, a missing or unreadable file throws `MediaError.Fetch`.
  public func mediaInfo(_ source: some MediaSource) async throws -> MediaInfo? {
    try await readMediaInfo(url: source.url)
  }
}

func readMediaInfo(url: String) async throws -> MediaInfo? {
  guard isLocalMediaUrl(url: url) else { return nil }
  guard let file = URL(string: url.trimmingCharacters(in: .whitespacesAndNewlines)), file.isFileURL,
        FileManager.default.isReadableFile(atPath: file.path)
  else { throw MediaError.Fetch(detail: "\(url) is not a readable file") }
  let reading = Task.detached { try await MediaInfoFile.read(file) }
  return try await withTaskCancellationHandler { try await reading.value } onCancel: { reading.cancel() }
}

private enum MediaInfoFile {
  static func read(_ file: URL) async throws -> MediaInfo? {
    let asset = AVURLAsset(url: file)
    guard let duration = try? await asset.load(.duration) else { return nil }
    try Task.checkCancellation()
    let tracks = (try? await asset.loadTracks(withMediaType: .video)) ?? []
    var frame: ImageSize?
    if let track = tracks.first, let (size, transform) = try? await track.load(.naturalSize, .preferredTransform) {
      let degrees = Int32((atan2(transform.b, transform.a) * 180 / .pi).rounded())
      frame = displayFrameSize(width: UInt32(abs(size.width)), height: UInt32(abs(size.height)), rotationDegrees: degrees)
    }
    try Task.checkCancellation()
    let millis = duration.seconds.isFinite && duration.seconds > 0 ? UInt64((duration.seconds * 1000).rounded()) : nil
    guard millis != nil || frame != nil else { return nil }
    return MediaInfo(durationMs: millis, frame: frame)
  }
}
