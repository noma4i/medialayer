import Foundation

final class FrameCache: @unchecked Sendable {
  struct Key: Hashable {
    let identity: String
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

  private struct Entry {
    let frames: AnimatedFrames
    let cost: Int
  }

  private let byteLimit: Int
  private let cost: @Sendable (AnimatedFrames) -> Int
  private let decode: @Sendable (Data, Int) async -> AnimatedFrames?
  private let lock = NSLock()
  private var entries: [Key: Entry] = [:]
  private var totalBytes = 0
  private var recency: [Key] = []
  private var flights: [Key: Flight] = [:]
  private var decodeCount = 0

  init(byteLimit: Int = defaultByteLimit, cost: @escaping @Sendable (AnimatedFrames) -> Int = { $0.decodedBytes }, decode: @escaping @Sendable (Data, Int) async -> AnimatedFrames? = { await AnimatedFrames.load($0, maxPixel: $1) }) {
    self.byteLimit = byteLimit
    self.cost = cost
    self.decode = decode
  }

  var bytes: Int { lock.withLock { totalBytes } }
  var count: Int { lock.withLock { entries.count } }
  var decodes: Int { lock.withLock { decodeCount } }
  var waiters: Int { lock.withLock { flights.values.reduce(0) { $0 + $1.waiters } } }

  func load(identity: String, data: Data, maxPixel: Int) async -> AnimatedFrames? {
    let key = Key(identity: identity, maxPixel: maxPixel)
    switch lock.withLock({ join(key, data: data, maxPixel: maxPixel) }) {
    case let .hit(frames): return frames
    case let .flight(flight): return await withTaskCancellationHandler { await flight.task.value } onCancel: { leave(flight, key) }
    }
  }

  func removeAll() {
    let pending = lock.withLock {
      entries.removeAll()
      totalBytes = 0
      recency.removeAll()
      let pending = Array(flights.values)
      flights.removeAll()
      return pending
    }
    pending.forEach { $0.task.cancel() }
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
      let frames = await decode(data, maxPixel)
      return lock.withLock {
        guard flights[key] === flight else { return nil }
        flights[key] = nil
        if let frames { store(frames, for: key) }
        return frames
      }
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
    return hit.frames
  }

  private func store(_ frames: AnimatedFrames, for key: Key) {
    let bytes = cost(frames)
    guard bytes > 0, bytes <= byteLimit else { return }

    totalBytes -= entries[key]?.cost ?? 0
    entries[key] = Entry(frames: frames, cost: bytes)
    totalBytes += bytes
    recency.removeAll { $0 == key }
    recency.append(key)
    while totalBytes > byteLimit, let oldest = recency.first {
      recency.removeFirst()
      if let removed = entries.removeValue(forKey: oldest) { totalBytes -= removed.cost }
    }
  }
}
