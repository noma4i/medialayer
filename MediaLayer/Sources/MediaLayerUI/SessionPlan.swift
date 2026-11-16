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

}

func hadPrivateOutput(_ ports: [AVAudioSession.Port]) -> Bool {
  let privatePorts: Set<AVAudioSession.Port> = [
    .headphones, .bluetoothA2DP, .bluetoothHFP, .bluetoothLE, .usbAudio,
    .carAudio, .airPlay, .lineOut, .HDMI
  ]
  return ports.contains { privatePorts.contains($0) }
}
