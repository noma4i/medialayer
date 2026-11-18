import Foundation

/// owner 2026-11-19: a start of the microphone (voice and camera) tries it up to `attempts` times. Each attempt waits at most `soundWaitMs` for the first live sound,
/// `pauseMs` pass between attempts. An attempt fails when it gives no sound in time, its source does not start or the system takes it before its first PCM.
/// The app sees only the outcome: the first sound of any attempt, or the refusal of the last one.
enum MicrophoneRetry {
  static let attempts = 3
  static let soundWaitMs: UInt64 = 1_000
  static let pauseMs: UInt64 = 200
}

/// The clock of the attempts: runs the work once after the delay unless the returned cancel runs first.
struct RetryTimer: Sendable {
  let schedule: @Sendable (_ delayMs: UInt64, _ work: @escaping @Sendable () -> Void) -> @Sendable () -> Void

  static let system = RetryTimer { delayMs, work in
    let item = DispatchWorkItem(block: work)
    DispatchQueue.global(qos: .userInitiated).asyncAfter(deadline: .now() + .milliseconds(Int(delayMs)), execute: item)
    return { item.cancel() }
  }
}
