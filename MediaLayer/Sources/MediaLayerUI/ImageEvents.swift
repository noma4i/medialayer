import Foundation
import MediaLayer
@_spi(AsyncImageDecoding) import Nuke
import os

final class ImageEvents: ImagePipeline.Delegate {
  static let log = OSLog(subsystem: "com.noma4i.medialayer", category: "MediaImage")
  let stats: ImageCounters

  init(stats: ImageCounters) { self.stats = stats }

  func imageDecoder(for context: ImageDecodingContext, pipeline: ImagePipeline) -> (any ImageDecoding)? {
    guard let decoder = pipeline.configuration.makeImageDecoder(context) else { return nil }
    if let asynchronous = decoder as? any AsyncImageDecoding { return AsyncImageDecode(decoder: asynchronous, stats: stats) }
    return ImageDecode(decoder: decoder, stats: stats)
  }

  func imageTaskCreated(_ task: ImageTask, pipeline: ImagePipeline) {
    os_signpost(.begin, log: Self.log, name: "MediaImage.request", signpostID: OSSignpostID(log: Self.log, object: task))
    stats.recordLater(work: .request(placementPixels: task.request.userInfo["placementPixels"] as? UInt64 ?? 0))
  }

  func imageTask(_ task: ImageTask, didReceiveEvent event: ImageTask.Event, pipeline: ImagePipeline) {
    guard case .finished(let result) = event else { return }
    os_signpost(.end, log: Self.log, name: "MediaImage.request", signpostID: OSSignpostID(log: Self.log, object: task))
    switch result {
    case .success(let response): stats.recordLater(work: .finish(memoryHit: response.cacheType == .memory))
    case .failure(.cancelled): stats.recordLater(work: .cancel)
    case .failure: stats.recordLater(work: .finish(memoryHit: false))
    }
  }
}

private struct ImageDecode: ImageDecoding {
  static let trace = OSSignposter(subsystem: "com.noma4i.medialayer", category: "MediaImage")
  let decoder: any ImageDecoding
  let stats: ImageCounters

  var isAsynchronous: Bool { decoder.isAsynchronous }

  func decode(_ data: Data) throws -> ImageContainer {
    let interval = Self.trace.beginInterval("MediaImage.decode", id: Self.trace.makeSignpostID())
    defer { Self.trace.endInterval("MediaImage.decode", interval) }
    let image = try decoder.decode(data)
    count(image)
    return image
  }

  func decodePartiallyDownloadedData(_ data: Data) -> ImageContainer? {
    let image = decoder.decodePartiallyDownloadedData(data)
    if let image { count(image) }
    return image
  }

  private func count(_ image: ImageContainer) { countDecode(image, stats: stats) }
}

private struct AsyncImageDecode: AsyncImageDecoding {
  let decoder: any AsyncImageDecoding
  let stats: ImageCounters

  func decode(_ data: Data) async throws -> ImageContainer {
    let interval = ImageDecode.trace.beginInterval("MediaImage.decode", id: ImageDecode.trace.makeSignpostID())
    defer { ImageDecode.trace.endInterval("MediaImage.decode", interval) }
    let image = try await decoder.decode(data)
    countDecode(image, stats: stats)
    return image
  }

  func decodePartiallyDownloadedData(_ data: Data) -> ImageContainer? {
    let image = decoder.decodePartiallyDownloadedData(data)
    if let image { countDecode(image, stats: stats) }
    return image
  }
}

private func countDecode(_ image: ImageContainer, stats: ImageCounters) {
  #if canImport(UIKit)
  guard let pixels = image.image.cgImage else { return }
  stats.recordLater(work: .decode(width: UInt32(pixels.width), height: UInt32(pixels.height), bytes: UInt64(pixels.bytesPerRow * pixels.height)))
  #endif
}
