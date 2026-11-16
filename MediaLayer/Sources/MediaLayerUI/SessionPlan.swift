@preconcurrency import AVFoundation
import MediaLayer

struct SessionPlan: Equatable {
  let category: AVAudioSession.Category
  let mode: AVAudioSession.Mode
  let options: AVAudioSession.CategoryOptions

  static func of(_ want: SessionWant) -> SessionPlan {
    switch want {
    case let .playback(focus):
      let options: AVAudioSession.CategoryOptions
      switch focus {
      case .exclusive: options = []
      case .mix: options = [.mixWithOthers]
      case .duck: options = [.mixWithOthers, .duckOthers]
      }
      return SessionPlan(category: .playback, mode: .moviePlayback, options: options)
    case let .recording(kind):
      return SessionPlan(category: .playAndRecord, mode: kind == .voice ? .default : .videoRecording, options: [.allowBluetoothA2DP])
    case .call:
      return SessionPlan(category: .playAndRecord, mode: .voiceChat, options: [])
    }
  }

  var trace: String { "\(category.rawValue) \(mode.rawValue) options=\(options.rawValue)" }

  static let recording = SessionPlan(category: .playAndRecord, mode: .default, options: [.defaultToSpeaker])

  static func call(speaker: Bool) -> SessionPlan {
    SessionPlan(category: .playAndRecord, mode: .voiceChat, options: speaker ? [.allowBluetoothHFP, .defaultToSpeaker] : [.allowBluetoothHFP])
  }

  static func playback(_ focus: MediaAudioFocus) -> SessionPlan {
    switch focus {
    case .exclusive: SessionPlan(category: .playback, mode: .moviePlayback, options: [])
    case .mix: SessionPlan(category: .playback, mode: .moviePlayback, options: [.mixWithOthers])
    case .duck: SessionPlan(category: .playback, mode: .moviePlayback, options: [.mixWithOthers, .duckOthers])
    }
  }
}

func hadPrivateOutput(_ ports: [AVAudioSession.Port]) -> Bool {
  let privatePorts: Set<AVAudioSession.Port> = [
    .headphones, .bluetoothA2DP, .bluetoothHFP, .bluetoothLE, .usbAudio,
    .carAudio, .airPlay, .lineOut, .HDMI
  ]
  return ports.contains { privatePorts.contains($0) }
}
