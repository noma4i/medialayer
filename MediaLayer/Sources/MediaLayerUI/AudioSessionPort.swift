import Foundation
import MediaLayer

enum SystemAudioEvent: Sendable {
  case interruptionBegan
  case interruptionEnded(shouldResume: Bool)
  case resumption(shouldResume: Bool)
  case mediaServicesReset
  case routeChanged(RouteReason, hadPrivateOutput: Bool)
}

struct AudioSessionBusy: Error {}

protocol AudioSessionPort: Sendable {
  func systemEvents(_ sink: @escaping @Sendable (SystemAudioEvent) -> Void) -> @Sendable () -> Void
  func apply(_ plan: SessionPlan, activate: Bool) throws
  func deactivate() throws
  func overrideSpeaker(_ on: Bool) throws
  func outputVolume() -> Float
  func volumeChanges() -> AsyncStream<Float>
}

protocol CallObserverPort: Sendable {
  func calls(_ sink: @escaping @Sendable (Set<String>) -> Void) -> @Sendable () -> Void
}

protocol MicrophonePermissionPort: Sendable {
  func granted() -> Bool
}

struct AudioPorts: Sendable {
  let session: any AudioSessionPort
  let calls: any CallObserverPort
  let permission: any MicrophonePermissionPort

  static func system(focus: AudioFocus) -> AudioPorts {
    AudioPorts(session: SystemAudioSession(focus: focus), calls: SystemCallObserver(), permission: SystemRecordingPermission())
  }
}
