import CryptoKit
import Foundation
import MediaLayer

/// Custody of the local original of an outgoing item, by its key: a hard link (a copy when the link fails) in a directory of the
/// MediaLayer cache root. It survives a recycled row and a relaunch, is released when the item is ready and swept once when a layer opens.
final class OutgoingStore: @unchecked Sendable {
  private struct Meta: Codable {
    var created: UInt64
    var lastDeclared: UInt64
  }

  let directory: URL
  private let lock = NSLock()
  private let now: @Sendable () -> UInt64

  init(directory: URL, now: @escaping @Sendable () -> UInt64 = { UInt64(Date().timeIntervalSince1970 * 1000) }) {
    self.directory = directory
    self.now = now
  }

  static func standard() -> OutgoingStore {
    let caches = (try? FileManager.default.url(for: .cachesDirectory, in: .userDomainMask, appropriateFor: nil, create: true))
      ?? FileManager.default.temporaryDirectory
    return OutgoingStore(directory: caches.appendingPathComponent("MediaLayer", isDirectory: true).appendingPathComponent("outgoing", isDirectory: true))
  }

  func folder(for key: String) -> URL {
    let digest = SHA256.hash(data: Data(key.utf8)).map { String(format: "%02x", $0) }.joined()
    return directory.appendingPathComponent(String(digest.prefix(32)), isDirectory: true)
  }

  /// The kept original of `key`, if one is stored.
  func stored(for key: String) -> URL? {
    lock.withLock {
      let folder = folder(for: key)
      let names = (try? FileManager.default.contentsOfDirectory(atPath: folder.path)) ?? []
      return names.first { $0 != "meta.json" }.map { folder.appendingPathComponent($0) }
    }
  }

  /// Keeps `local` under `key` and answers the kept file. A second call for a key that is already kept only renews its declaration time.
  @discardableResult
  func keep(_ local: URL, for key: String) -> URL? {
    lock.withLock {
      let folder = folder(for: key)
      let manager = FileManager.default
      try? manager.createDirectory(at: folder, withIntermediateDirectories: true)
      let stamp = now()
      var meta = readMeta(folder) ?? Meta(created: stamp, lastDeclared: stamp)
      meta.lastDeclared = stamp
      writeMeta(meta, to: folder)
      if let kept = (try? manager.contentsOfDirectory(atPath: folder.path))?.first(where: { $0 != "meta.json" }) {
        return folder.appendingPathComponent(kept)
      }
      let ext = local.pathExtension
      let target = folder.appendingPathComponent(ext.isEmpty ? "original" : "original.\(ext)")
      do {
        try manager.linkItem(at: local, to: target)
      } catch {
        do { try manager.copyItem(at: local, to: target) } catch { return nil }
      }
      return target
    }
  }

  func release(_ key: String) {
    lock.withLock { try? FileManager.default.removeItem(at: folder(for: key)) }
  }

  /// One pass: removes every kept original that the core calls stale. Run once when a layer opens.
  func collectStale() {
    lock.withLock {
      let manager = FileManager.default
      let folders = (try? manager.contentsOfDirectory(at: directory, includingPropertiesForKeys: nil)) ?? []
      let stamp = now()
      for folder in folders {
        guard let meta = readMeta(folder) else {
          try? manager.removeItem(at: folder)
          continue
        }
        if outgoingStale(createdMs: meta.created, lastDeclaredMs: meta.lastDeclared, nowMs: stamp, ready: false) {
          try? manager.removeItem(at: folder)
        }
      }
    }
  }

  private func readMeta(_ folder: URL) -> Meta? {
    (try? Data(contentsOf: folder.appendingPathComponent("meta.json"))).flatMap { try? JSONDecoder().decode(Meta.self, from: $0) }
  }

  private func writeMeta(_ meta: Meta, to folder: URL) {
    try? JSONEncoder().encode(meta).write(to: folder.appendingPathComponent("meta.json"), options: .atomic)
  }
}
