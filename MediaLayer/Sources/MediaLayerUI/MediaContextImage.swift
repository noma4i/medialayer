import MediaLayer
import Nuke
import SwiftUI

extension MediaContext {
  public func image(for source: some MediaSource, points: CGSize, scale: CGFloat) async -> PlatformImage? {
    guard !Task.isCancelled else { return nil }
    guard let request = await MediaImagePipeline.prepareOffMain(places: places, for: source, points: points, scale: scale).request else { return nil }
    return await MediaImagePipeline.load(source, request: request, pipeline: pipeline)?.image
  }
}
