import MediaLayer
import Nuke
import SwiftUI

extension MediaContext {
  public func image(for source: some MediaSource, points: CGSize, scale: CGFloat) async -> PlatformImage? {
    guard !Task.isCancelled, let request = MediaImagePipeline.request(places: places, for: source, points: points, scale: scale) else { return nil }
    return await MediaImagePipeline.load(source, request: request, pipeline: pipeline)?.image
  }
}
