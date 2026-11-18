import Foundation
import MediaLayer
import MediaLayerUI

/// Feeds system audio events into a real opened `MediaLayer`, through the same paths the system signals take,
/// so `VoiceState`, `CameraState` and `microphoneAvailability` run their real code without a real call.
/// Test targets only: the entry points are `package` and not visible to an app.
public struct MediaAudioDriver: Sendable {
  private let media: MediaLayer

  public init(_ media: MediaLayer) { self.media = media }

  /// The microphone stops delivering sound: live takes end with `microphoneSilent`, availability becomes `NoSound`.
  public func silenceMicrophone() { media.driveAudio(.silenceMicrophone) }
  /// Sound flows again.
  public func restoreMicrophone() { media.driveAudio(.restoreMicrophone) }
  /// A call of another app starts: takes end with `inCall`, availability becomes `externalCall`.
  public func startCall() { media.driveAudio(.startCall) }
  public func endCall() { media.driveAudio(.endCall) }
  public func beginInterruption() { media.driveAudio(.beginInterruption) }
  public func endInterruption(shouldResume: Bool = true) { media.driveAudio(.endInterruption(shouldResume: shouldResume)) }
  /// Changes only the library state, not the system permission.
  public func revokePermission() { media.driveAudio(.permission(.denied)) }
  /// Changes only the library state, not the system permission.
  public func grantPermission() { media.driveAudio(.permission(.granted)) }
}
