import Foundation
import MediaLayer

/// System audio events a test feeds through the real `AudioLink`. Reachable only from this package (`MediaLayerTesting`).
package enum AudioDriveSignal: Sendable {
  case silenceMicrophone
  case restoreMicrophone
  case startCall
  case endCall
  case beginInterruption
  case endInterruption(shouldResume: Bool)
  case permission(MicrophonePermission)
}

extension AudioLink {
  /// Calls the same entry points the system ports call, so the core and the wrappers run their real paths.
  func drive(_ signal: AudioDriveSignal) {
    switch signal {
    case .silenceMicrophone:
      inbox.async {
        self.tickets.keys.forEach { self.process(.silenced(id: $0)) }
        self.inputRegistry.forEach { id, input in
          if let generation = input.generation { self.process(.noSound(id: id, generation: generation, on: true)) }
        }
      }
    case .restoreMicrophone:
      inbox.async {
        self.inputRegistry.forEach { id, input in
          if let generation = input.generation { self.process(.noSound(id: id, generation: generation, on: false)) }
        }
      }
    case .startCall: observedCalls([Self.testingCallID])
    case .endCall: observedCalls([])
    case .beginInterruption: systemEvent(.interruptionBegan)
    case let .endInterruption(shouldResume): systemEvent(.interruptionEnded(shouldResume: shouldResume))
    case let .permission(state): send(.permission(state: state))
    }
  }

  static let testingCallID = "MediaLayerTesting.external-call"
}

extension MediaLayer {
  package func driveAudio(_ signal: AudioDriveSignal) {
    (objc_getAssociatedObject(self, &lifetimeKey) as? SystemSignals)?.audio?.drive(signal)
  }
}
