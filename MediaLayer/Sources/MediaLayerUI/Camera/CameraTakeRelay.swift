#if os(iOS)
import CoreMedia
import Foundation
import MediaLayer

/// Callbacks of a take that is no longer the live one are dropped, so each event reaches the camera once.
final class CameraTakeRelay: @unchecked Sendable {
  private let queue: DispatchQueue
  private let emit: @Sendable (CaptureEvent) -> Void
  private let idle: @Sendable () -> Void
  private let lock = NSLock()
  private var take: CameraTake?
  private var lastGeneration: UInt64 = 0
  private var finishedCoverage: AudioCoverage?

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
      finishedCoverage = nil
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

  /// The verdict of the take that finished last, once: the camera reads it when the take ends.
  func takeCoverage() -> AudioCoverage? {
    lock.withLock { defer { finishedCoverage = nil }; return finishedCoverage }
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

  /// The microphone is gone: a take that waits for the sound ends with what it has.
  func audioEnded() {
    lock.withLock { take }?.audioEnded()
  }

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
    emit(.audioFlow(flowing: true))
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
    case let .finished(_, _, reachedLimit, coverage):
      lock.withLock { finishedCoverage = coverage }
      emit(.recordFinished(end: reachedLimit ? .limitReached : .completed))
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
