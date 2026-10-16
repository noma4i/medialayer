import Foundation

final class FrameCache: @unchecked Sendable {
  struct Key: Hashable {
    let url: String
    let maxPixel: Int
  }

  static let defaultByteLimit = 64 << 20
  static let shared = FrameCache()

  private final class Flight {
    var task: Task<AnimatedFrames?, Never>!
    var waiters = 1
  }

  private enum Joined {
    case hit(AnimatedFrames)
    case flight(Flight)
  }

  private let byteLimit: Int
  private let lock = NSLock()
  private var entries: [Key: AnimatedFrames] = [:]
  private var recency: [Key] = []
  private var flights: [Key: Flight] = [:]
  private var decodeCount = 0

  init(byteLimit: Int = defaultByteLimit) {
    self.byteLimit = byteLimit
  }

  var bytes: Int { lock.withLock { entries.values.reduce(0) { $0 + $1.decodedBytes } } }
  var count: Int { lock.withLock { entries.count } }
  var decodes: Int { lock.withLock { decodeCount } }

  func load(url: String, data: Data, maxPixel: Int) async -> AnimatedFrames? {
    let key = Key(url: url, maxPixel: maxPixel)
    switch lock.withLock({ join(key, data: data, maxPixel: maxPixel) }) {
    case let .hit(frames): return frames
    case let .flight(flight): return await withTaskCancellationHandler { await flight.task.value } onCancel: { leave(flight, key) }
    }
  }

  func removeAll() {
    lock.withLock {
      entries.removeAll()
      recency.removeAll()
    }
  }

  private func join(_ key: Key, data: Data, maxPixel: Int) -> Joined {
    if let hit = touch(key) { return .hit(hit) }
    if let running = flights[key] {
      running.waiters += 1
      return .flight(running)
    }

    decodeCount += 1
    let flight = Flight()
    flight.task = Task {
      let frames = await AnimatedFrames.load(data, maxPixel: maxPixel)
      lock.withLock {
        if flights[key] === flight { flights[key] = nil }
        if let frames { store(frames, for: key) }
      }
      return frames
    }
    flights[key] = flight
    return .flight(flight)
  }

  private func leave(_ flight: Flight, _ key: Key) {
    let abandoned = lock.withLock { () -> Bool in
      guard flights[key] === flight else { return false }

      flight.waiters -= 1
      guard flight.waiters == 0 else { return false }

      flights[key] = nil
      return true
    }
    if abandoned { flight.task.cancel() }
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
