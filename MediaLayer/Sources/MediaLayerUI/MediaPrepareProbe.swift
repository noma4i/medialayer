@preconcurrency import AVFoundation
import Foundation
import ImageIO
import MediaLayer
import UniformTypeIdentifiers

/// What the platform reads from a source: the facts the core decides on. A source the device cannot read or decode is `.failed`.
enum PrepareFacts {
  /// A known descriptor `kind` picks the reader without sniffing; without one the mime hint or the file decides.
  static func read(_ file: URL, mime: String?, kind: PrepareKind? = nil) async -> PrepareProbe {
    if kind == .video { return await video(file) }
    if kind == .image { return image(file) ?? .failed(message: "\(file.path) is not an image") }
    if let mime, mime.hasPrefix("image/") { return image(file) ?? .failed(message: "\(file.path) is not an image") }
    if let mime, mime.hasPrefix("video/") { return await video(file) }
    if let picture = image(file) { return picture }
    return await video(file)
  }

  private static func image(_ file: URL) -> PrepareProbe? {
    guard let source = CGImageSourceCreateWithURL(file as CFURL, nil),
          let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
          let width = properties[kCGImagePropertyPixelWidth] as? Int,
          let height = properties[kCGImagePropertyPixelHeight] as? Int
    else { return nil }
    return .image(
      width: UInt32(clamping: width), height: UInt32(clamping: height),
      exifOrientation: (properties[kCGImagePropertyOrientation] as? UInt32).flatMap { (1...8).contains($0) ? $0 : nil } ?? 1,
      format: (CGImageSourceGetType(source) as String?) == UTType.png.identifier ? .png : .jpeg
    )
  }

  private static func video(_ file: URL) async -> PrepareProbe {
    do {
      let asset = PrepareContainer.asset(file)
      guard let track = try await asset.loadTracks(withMediaType: .video).first else {
        return .failed(message: "\(file.path) has no video track")
      }
      let (size, transform, rate, dataRate, formats, decodable) = try await track.load(
        .naturalSize, .preferredTransform, .nominalFrameRate, .estimatedDataRate, .formatDescriptions, .isDecodable
      )
      let audioTrack = try await asset.loadTracks(withMediaType: .audio).first
      let audioFormats = try await audioTrack?.load(.formatDescriptions) ?? []
      let fits = PrepareContainer.isMp4(file)
        && formats.allSatisfy { CMFormatDescriptionGetMediaSubType($0) == kCMVideoCodecType_H264 }
        && audioFormats.allSatisfy { CMFormatDescriptionGetMediaSubType($0) == kAudioFormatMPEG4AAC }
      if !fits, !decodable { return .failed(message: "the device cannot decode the \(Int(abs(size.width)))x\(Int(abs(size.height))) source") }
      let source = VideoSource(
        width: UInt32(abs(size.width)), height: UInt32(abs(size.height)),
        rotationDegrees: (Int32((atan2(transform.b, transform.a) * 180 / .pi).rounded()) % 360 + 360) % 360,
        frameRate: rate, bitrate: dataRate > 0 ? UInt32(dataRate) : nil, mp4H264Aac: fits
      )
      let bytes = (try? FileManager.default.attributesOfItem(atPath: file.path)[.size] as? UInt64) ?? 0
      return .video(source: source, metadataKeys: try await metadataKeys(asset, tracks: [track] + (audioTrack.map { [$0] } ?? [])), sizeBytes: bytes)
    } catch {
      return .failed(message: "\(file.path) is not a video: \(error.localizedDescription)")
    }
  }

  private static func metadataKeys(_ asset: AVURLAsset, tracks: [AVAssetTrack]) async throws -> [String] {
    var items: [AVMetadataItem] = []
    for format in try await asset.load(.availableMetadataFormats) { items += try await asset.loadMetadata(for: format) }
    for track in tracks { items += try await track.load(.metadata) }
    return items.compactMap { ($0.identifier?.rawValue ?? ($0.key as? String)).map { locationKey(decodedKey($0)) } }
  }

  private static func locationKey(_ key: String) -> String {
    let name = (key.split(separator: "/", maxSplits: 1).last.map(String.init) ?? key).lowercased()
    return name == "loci" || name == "©xyz" || name.hasPrefix("com.apple.quicktime.location.") ? "location" : key
  }

  /// AVFoundation escapes the bytes of a four-character key, so ©too arrives as `%A9too`.
  private static func decodedKey(_ identifier: String) -> String {
    var bytes: [UInt8] = []
    var rest = Array(identifier.utf8)[...]
    while let byte = rest.popFirst() {
      let escaped = byte == UInt8(ascii: "%") ? rest.prefix(2).compactMap { Character(UnicodeScalar($0)).hexDigitValue } : []
      if escaped.count == 2 {
        bytes.append(UInt8(escaped[0] * 16 + escaped[1]))
        rest = rest.dropFirst(2)
      } else {
        bytes.append(byte)
      }
    }
    return String(bytes: bytes, encoding: .utf8) ?? String(String.UnicodeScalarView(bytes.map(UnicodeScalar.init)))
  }
}

enum PrepareContainer {
  static func asset(_ url: URL) -> AVURLAsset {
    guard let brand = brand(url) else { return AVURLAsset(url: url) }
    return AVURLAsset(url: url, options: [AVURLAssetOverrideMIMETypeKey: brand == "qt  " ? "video/quicktime" : "video/mp4"])
  }

  static func isMp4(_ url: URL) -> Bool {
    guard let brand = brand(url) else { return false }
    return brand != "qt  "
  }

  private static func brand(_ url: URL) -> String? {
    guard let handle = try? FileHandle(forReadingFrom: url), let head = try? handle.read(upToCount: 12) else { return nil }
    try? handle.close()
    guard head.count == 12, head[4..<8].elementsEqual("ftyp".utf8) else { return nil }
    return String(decoding: head[8..<12], as: UTF8.self)
  }
}
