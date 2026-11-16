@preconcurrency import AVFoundation

struct SessionPlan: Equatable {
  let category: AVAudioSession.Category
  let mode: AVAudioSession.Mode
  let options: AVAudioSession.CategoryOptions

  var trace: String { "\(category.rawValue) \(mode.rawValue) options=\(options.rawValue)" }

  static let recording = SessionPlan(category: .playAndRecord, mode: .default, options: [.defaultToSpeaker])

  static func playback(_ focus: MediaAudioFocus) -> SessionPlan {
    switch focus {
    case .exclusive: SessionPlan(category: .playback, mode: .moviePlayback, options: [])
    case .mix: SessionPlan(category: .playback, mode: .moviePlayback, options: [.mixWithOthers])
    case .duck: SessionPlan(category: .playback, mode: .moviePlayback, options: [.mixWithOthers, .duckOthers])
    }
  }
}
