@preconcurrency import AVFoundation

struct SystemAudioSession: AudioSessionPort {
  func setCategory(_ plan: SessionPlan) throws {
    try AVAudioSession.sharedInstance().setCategory(plan.category, mode: plan.mode, options: plan.options)
  }

  func setActive(_ active: Bool) throws {
    try AVAudioSession.sharedInstance().setActive(active, options: active ? [] : [.notifyOthersOnDeactivation])
  }
}
