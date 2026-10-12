import Foundation

final class FrameCache: @unchecked Sendable {
  struct Key: Hashable {
    let url: String
    let maxPixel: Int
  }

  static let defaultByteLimit = 64 << 20
  static let shared = FrameCache()

  private let byteLimit: Int
  private let lock = NSLock()
  private var entries: [Key: AnimatedFrames] = [:]
  private var recency: [Key] = []
  private var decodeCount = 0

  init(byteLimit: Int = defaultByteLimit) {
    self.byteLimit = byteLimit
  }

  var bytes: Int { lock.withLock { entries.values.reduce(0) { $0 + $1.decodedBytes } } }
  var count: Int { lock.withLock { entries.count } }
  var decodes: Int { lock.withLock { decodeCount } }

  func load(url: String, data: Data, maxPixel: Int) async -> AnimatedFrames? {
    let key = Key(url: url, maxPixel: maxPixel)
    if let hit = lock.withLock({ touch(key) }) { return hit }

    lock.withLock { decodeCount += 1 }
    guard let frames = await AnimatedFrames.load(data, maxPixel: maxPixel) else { return nil }

    lock.withLock { store(frames, for: key) }
    return frames
  }

  func removeAll() {
    lock.withLock {
      entries.removeAll()
      recency.removeAll()
    }
  }

  private func touch(_ key: Key) -> AnimatedFrames? {
    guard let hit = entries[key] else { return nil }

    recency.removeAll { $0 == key }
    recency.append(key)
    return hit
  }

  private func store(_ frames: AnimatedFrames, for key: Key) {
    guard frames.decodedBytes > 0, frames.decodedBytes <= byteLimit else { return }

    entries[key] = frames
    recency.removeAll { $0 == key }
    recency.append(key)
    while entries.values.reduce(0, { $0 + $1.decodedBytes }) > byteLimit, let oldest = recency.first {
      recency.removeFirst()
      entries[oldest] = nil
    }
  }
}
