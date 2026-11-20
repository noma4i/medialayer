import Foundation
import ImageIO
import MediaLayer

/// Fetches the remote copy of an outgoing image into the layer cache and says what came of it. `Complete` means the whole body is
/// cached and decodes.
public protocol OutgoingRemote: Sendable {
  func fetch(url: String, headers: [String: String]) async -> RemoteResult
}

/// The timer of the 5 s re-ask. Tests drive it by hand.
public protocol OutgoingClock: Sendable {
  func sleep(milliseconds: UInt32) async throws
}

struct SystemOutgoingClock: OutgoingClock {
  func sleep(milliseconds: UInt32) async throws { try await Task.sleep(for: .milliseconds(Int(milliseconds))) }
}

/// The image pipeline's own path: the complete-body cache of the core, then a check that the body decodes.
struct LayerOutgoingRemote: OutgoingRemote {
  let places: @Sendable () -> MediaPlaces?

  func fetch(url: String, headers: [String: String]) async -> RemoteResult {
    guard let places = places() else { return .failed(status: nil, detail: "the media layer is not open") }
    do {
      let fetched = try await places.fetchImage(url: url, headers: headers)
      return Self.decodes(fetched.bytes) ? .complete : .failed(status: nil, detail: "the picture does not decode")
    } catch {
      return Self.result(of: error)
    }
  }

  static func decodes(_ bytes: Data) -> Bool {
    guard let source = CGImageSourceCreateWithData(bytes as CFData, nil), CGImageSourceGetCount(source) > 0 else { return false }
    let options: [CFString: Any] = [kCGImageSourceCreateThumbnailFromImageAlways: true, kCGImageSourceThumbnailMaxPixelSize: 32]
    return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary) != nil
  }

  /// A 404, 202 or 425 from the origin means the server is still working on the copy.
  static func result(of error: Error) -> RemoteResult {
    let detail: String
    if case MediaError.Fetch(let text) = error { detail = text } else { detail = error.localizedDescription }
    let status = statusCode(in: detail)
    if let status, [404, 202, 425].contains(status) { return .notReady }
    return .failed(status: status, detail: detail)
  }

  static func statusCode(in detail: String) -> UInt16? {
    guard let range = detail.range(of: "status ") else { return nil }
    let digits = detail[range.upperBound...].prefix { $0.isNumber }
    return UInt16(digits)
  }
}

/// At most `outgoingFetchSlots()` outgoing fetches run at once, the rest wait in order.
@MainActor
final class OutgoingFetchSlots {
  private var running = 0
  private var waiters: [(id: UInt64, resume: CheckedContinuation<Bool, Never>)] = []
  private var next: UInt64 = 0
  let limit: Int

  init(limit: Int = Int(outgoingFetchSlots())) { self.limit = max(1, limit) }

  var active: Int { running }

  func acquire() async -> Bool {
    if running < limit {
      running += 1
      return true
    }
    next += 1
    let id = next
    return await withTaskCancellationHandler {
      await withCheckedContinuation { continuation in
        waiters.append((id, continuation))
      }
    } onCancel: {
      Task { @MainActor in self.cancel(id) }
    }
  }

  private func cancel(_ id: UInt64) {
    guard let index = waiters.firstIndex(where: { $0.id == id }) else { return }
    waiters.remove(at: index).resume.resume(returning: false)
  }

  func release() {
    if waiters.isEmpty {
      running -= 1
    } else {
      waiters.removeFirst().resume.resume(returning: true)
    }
  }
}

private final class WeakEntry {
  weak var entry: OutgoingEntry?
  init(_ entry: OutgoingEntry) { self.entry = entry }
}

/// The registry: one entry per key. A busy entry is held here so its fetch continues with no view; a finished one is held weakly.
@MainActor
final class OutgoingRuntime {
  static let shared = OutgoingRuntime()

  var remote: OutgoingRemote = LayerOutgoingRemote(places: { ProcessMediaContext.shared.current?.places })
  var clock: OutgoingClock = SystemOutgoingClock()
  var store = OutgoingStore.standard()
  let slots = OutgoingFetchSlots()
  private var held: [String: OutgoingEntry] = [:]
  private var released: [String: WeakEntry] = [:]

  func entry(for key: String) -> OutgoingEntry? { held[key] ?? released[key]?.entry }

  func register(_ entry: OutgoingEntry) { held[entry.key] = entry }

  func finished(_ entry: OutgoingEntry) {
    guard held[entry.key] === entry else { return }
    held[entry.key] = nil
    released[entry.key] = WeakEntry(entry)
  }

  /// Test seam: the remote fetcher, the clock and the custody store behind every outgoing item.
  func use(remote: OutgoingRemote? = nil, clock: OutgoingClock? = nil, store: OutgoingStore? = nil) {
    if let remote { self.remote = remote }
    if let clock { self.clock = clock }
    if let store { self.store = store }
  }

  /// Test seam for `MediaLayerTesting`: the fakes behind every item over a custody store in `directory`.
  package func installForTesting(remote: OutgoingRemote, clock: OutgoingClock, directory: URL) {
    forgetAll()
    use(remote: remote, clock: clock, store: OutgoingStore(directory: directory))
  }

  /// Test seam for `MediaLayerTesting`: forgets every item and puts the system remote, clock and store back.
  package func resetForTesting() {
    forgetAll()
    remote = LayerOutgoingRemote(places: { ProcessMediaContext.shared.current?.places })
    clock = SystemOutgoingClock()
    store = OutgoingStore.standard()
  }

  /// Test seam: a process starts with no entries, as after a relaunch. Kept originals stay on disk.
  func forgetAll() {
    for entry in held.values { entry.stop() }
    held = [:]
    released = [:]
  }

  /// One pass over the kept originals, run when a layer opens.
  func collectStale() {
    let store = store
    Task.detached(priority: .utility) { store.collectStale() }
  }
}

/// Reachable only from this package (`MediaLayerTesting`).
package enum OutgoingTestingHook {
  @MainActor package static func install(remote: OutgoingRemote, clock: OutgoingClock, directory: URL) {
    OutgoingRuntime.shared.installForTesting(remote: remote, clock: clock, directory: directory)
  }

  @MainActor package static func reset() { OutgoingRuntime.shared.resetForTesting() }
}
