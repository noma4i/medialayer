@preconcurrency import AVFoundation

struct SystemAudioSession: AudioSessionPort {
  func setCategory(_ plan: SessionPlan) throws {
    try AVAudioSession.sharedInstance().setCategory(plan.category, mode: plan.mode, options: plan.options)
  }

  func setActive(_ active: Bool) throws {
    try AVAudioSession.sharedInstance().setActive(active, options: active ? [] : [.notifyOthersOnDeactivation])
  }

  func outputVolume() -> Float {
    AVAudioSession.sharedInstance().outputVolume
  }

  func volumeChanges() -> AsyncStream<Float> {
    AsyncStream { continuation in
      let token = VolumeObservation(AVAudioSession.sharedInstance().observe(\.outputVolume, options: [.new]) { _, change in
        if let volume = change.newValue { continuation.yield(volume) }
      })
      continuation.onTermination = { _ in token.invalidate() }
    }
  }
}

private final class VolumeObservation: @unchecked Sendable {
  private let token: NSKeyValueObservation

  init(_ token: NSKeyValueObservation) {
    self.token = token
  }

  func invalidate() {
    token.invalidate()
  }
}
