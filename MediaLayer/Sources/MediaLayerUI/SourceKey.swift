import Foundation
import MediaLayer

/// The key of the core for a source, read off the main thread. A lookup by the declared text never happens.
enum SourceKey {
  static let core: @Sendable (MediaItem) -> String = { normalizeItem(item: $0).url }
  private static let seam = Seam()

  private final class Seam: @unchecked Sendable {
    private let lock = NSLock()
    private var current: @Sendable (MediaItem) -> String = SourceKey.core

    var read: @Sendable (MediaItem) -> String {
      get { lock.withLock { current } }
      set { lock.withLock { current = newValue } }
    }
  }

  static var read: @Sendable (MediaItem) -> String {
    get { seam.read }
    set { seam.read = newValue }
  }

  static func of(_ item: MediaItem) async -> String {
    let read = read
    return await Task.detached { read(item) }.value
  }
}
