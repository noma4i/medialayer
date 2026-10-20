import MediaLayer
import Nuke
import SwiftUI

extension MediaContext {
  public func image(for source: some MediaSource, points: CGSize, scale: CGFloat) async -> PlatformImage? {
    guard !Task.isCancelled, let request = MediaImagePipeline.request(places: places, for: source, points: points, scale: scale) else { return nil }
    if let hit = MediaImagePipeline.cachedContainer(for: request, in: pipeline) {
      places.itemState(key: source.url, state: .ready)
      return hit.image
    }
    return await MediaImagePipeline.load(source, request: request, places: places, pipeline: pipeline)?.image
  }
}
