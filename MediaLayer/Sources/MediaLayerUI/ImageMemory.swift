import MediaLayer
import Nuke
import ObjectiveC

nonisolated(unsafe) private var cacheKey: UInt8 = 0
nonisolated(unsafe) private var pipelineKey: UInt8 = 0

enum ImageMemory {
  static func pipeline(for places: MediaPlaces) -> ImagePipeline {
    objc_sync_enter(places)
    defer { objc_sync_exit(places) }
    if let existing = objc_getAssociatedObject(places, &pipelineKey) as? ImagePipeline { return existing }
    let pipeline = MediaImagePipeline.make(places: places)
    objc_setAssociatedObject(places, &pipelineKey, pipeline, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
    return pipeline
  }

  static func cache(for places: MediaPlaces) -> ImageCache {
    objc_getAssociatedObject(places, &cacheKey) as? ImageCache ?? ImageCache.shared
  }

  static func install(_ cache: ImageCache, on places: MediaPlaces) {
    objc_setAssociatedObject(places, &cacheKey, cache, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
  }
}
