#if os(iOS)
import CoreMedia
import Foundation
import MediaLayer

/// Owns the one live `CameraTake` of a camera backend and turns its callbacks into the events of the camera. Callbacks run on `queue`, the session queue of the backend.
/// A callback of a take that is no longer the live one is dropped, so a begin, a start or an outcome reaches the camera for the live take only and at most once.
final class CameraTakeRelay: @unchecked Sendable {
  private let queue: DispatchQueue
  private let emit: @Sendable (CaptureEvent) -> Void
  private let idle: @Sendable () -> Void
  private let lock = NSLock()
  private var take: CameraTake?
  private var lastGeneration: UInt64 = 0

  /// `idle` runs on `queue` when a live take ends or is refused, never for `cancel`.
  init(queue: DispatchQueue, emit: @escaping @Sendable (CaptureEvent) -> Void, idle: @escaping @Sendable () -> Void) {
    self.queue = queue
    self.emit = emit
    self.idle = idle
  }

  var isActive: Bool { lock.withLock { take != nil } }
  var recordedMs: UInt64 { lock.withLock { take }?.recordedMs ?? 0 }

  func begin(url: URL, settings: CameraTake.Settings, ticket: RecordingTicket?, refused: @escaping @Sendable (RecordingRefusal) -> Void) {
    let generation = lock.withLock { () -> UInt64 in
      lastGeneration &+= 1
      return lastGeneration
    }
    let take = CameraTake(
      url: url, generation: generation, settings: settings,
      onBegin: { [weak self] generation, _ in self?.queue.async { self?.confirm(generation, ticket: ticket, refused: refused) } },
      onStarted: { [weak self] in self?.queue.async { self?.started(generation, ticket: ticket) } },
      onOutcome: { [weak self] outcome in self?.queue.async { self?.settled(generation, outcome) } }
    )
    lock.withLock { self.take = take }
  }

  func appendAudio(_ buffer: CMSampleBuffer) {
    guard let take = lock.withLock({ take }) else { return }
    take.appendAudio(buffer, generation: take.generation)
  }

  func appendVideo(_ buffer: CMSampleBuffer) {
    guard let take = lock.withLock({ take }) else { return }
    take.appendVideo(buffer, generation: take.generation)
  }

  func stop() {
    lock.withLock { take }?.stop()
  }

  /// Ends the live take with no event and no file.
  func cancel() {
    lock.withLock { () -> CameraTake? in
      defer { take = nil }
      return take
    }?.cancel()
  }

  private func live(_ generation: UInt64) -> CameraTake? {
    lock.withLock { take?.generation == generation ? take : nil }
  }

  private func confirm(_ generation: UInt64, ticket: RecordingTicket?, refused: @Sendable (RecordingRefusal) -> Void) {
    guard let take = live(generation) else { return }
    do {
      if let ticket { try ticket.start { take.confirmBegin(generation: generation) } }
      else { take.confirmBegin(generation: generation) }
    } catch let reason as RecordingRefusal {
      drop(take)
      refused(reason)
    } catch {
      Media.recordNativeFailure(error, source: .camera)
      drop(take)
      refused(.interrupted)
    }
  }

  private func started(_ generation: UInt64, ticket: RecordingTicket?) {
    guard let take = live(generation) else { return }
    emit(.recordStarted)
    if let ticket, !ticket.isActive { take.stop() }
  }

  private func settled(_ generation: UInt64, _ outcome: CameraTake.Outcome) {
    guard live(generation) != nil else { return }
    lock.withLock { take = nil }
    switch outcome {
    case let .finished(_, _, reachedLimit): emit(.recordFinished(end: reachedLimit ? .limitReached : .completed))
    case let .failed(error):
      Media.recordNativeFailure(error, source: .camera)
      emit(.recordFinished(end: .failed))
    case .cancelled: break
    }
    idle()
  }

  private func drop(_ take: CameraTake) {
    lock.withLock { if self.take === take { self.take = nil } }
    take.cancel()
    idle()
  }
}
#endif
