import MediaLayer
import Nuke
import ObjectiveC

nonisolated(unsafe) private var cacheKey: UInt8 = 0

enum ImageMemory {
  static func cache(for places: MediaPlaces) -> ImageCache {
    objc_getAssociatedObject(places, &cacheKey) as? ImageCache ?? ImageCache.shared
  }

  static func install(_ cache: ImageCache, on places: MediaPlaces) {
    objc_setAssociatedObject(places, &cacheKey, cache, .OBJC_ASSOCIATION_RETAIN_NONATOMIC)
  }
}
