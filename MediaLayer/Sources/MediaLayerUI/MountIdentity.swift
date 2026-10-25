import MediaLayer

/// The full identity of a mounted content: another item (headers, strategy, cover, loop) or another context mounts a new place.
struct MountIdentity: Hashable {
  let item: MediaItem
  let context: ObjectIdentifier?

  init(item: MediaItem, context: MediaContext?) {
    self.item = item
    self.context = context.map { ObjectIdentifier($0.places) }
  }
}

/// A failure belongs to the full item of the mount that raised it; another item of the same URL does not inherit it.
struct ScopedFailure: Equatable {
  let item: MediaItem
  let state: MediaState

  func of(_ other: MediaItem) -> MediaState? { item == other ? state : nil }
}
