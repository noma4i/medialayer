enum SystemAudioEvent: Sendable {
  case interruptionBegan, interruptionEnded, mediaServicesReset, routeChanged
}

protocol AudioSessionPort: Sendable {
  func currentPlan() -> SessionPlan
  func systemEvents() -> AsyncStream<SystemAudioEvent>
  func setCategory(_ plan: SessionPlan) throws
  func setActive(_ active: Bool) throws
  var systemChanges: Int { get }
  func outputVolume() -> Float
  func volumeChanges() -> AsyncStream<Float>
}
