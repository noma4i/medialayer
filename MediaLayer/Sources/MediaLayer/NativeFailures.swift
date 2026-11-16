import Foundation

package enum NativeFailures {
  private static let lock = NSLock()
  nonisolated(unsafe) private static var sinks: [UUID: AsyncStream<NativeFailure>.Continuation] = [:]
  nonisolated(unsafe) private static var latest: NativeFailure?
  nonisolated(unsafe) private static var latestError: MediaError?

  package static var last: NativeFailure? { lock.withLock { latest } }
  package static var lastError: MediaError? { lock.withLock { latestError } }

  package static func record(_ error: MediaError) { lock.withLock { latestError = error } }

  package static var events: AsyncStream<NativeFailure> {
    AsyncStream { continuation in
      let id = UUID()
      lock.withLock { sinks[id] = continuation }
      continuation.onTermination = { _ in lock.withLock { sinks[id] = nil } }
    }
  }

  package static func publish(_ failure: NativeFailure) {
    lock.withLock {
      latest = failure
      latestError = nil
      sinks.values.forEach { $0.yield(failure) }
    }
  }

  package static func clear() { lock.withLock { latest = nil; latestError = nil } }

  package static func removeFile(_ url: URL) {
    if let failure = deleteFile(url) { publish(failure) }
  }

  package static func deleteFile(_ url: URL) -> NativeFailure? {
    guard FileManager.default.fileExists(atPath: url.path) else { return nil }
    do { try FileManager.default.removeItem(at: url) }
    catch {
      let native = error as NSError
      return NativeFailure(source: .file, domain: native.domain, code: Int64(native.code), message: native.localizedDescription)
    }
    return nil
  }
}
