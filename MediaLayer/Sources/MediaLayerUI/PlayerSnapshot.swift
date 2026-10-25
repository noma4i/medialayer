public struct PlayerSnapshot: Equatable, Sendable {
  public var key: String
  public var isPlaying: Bool
  public var isMuted: Bool
  public var positionMs: UInt64
  public var rate: Float

  public init(key: String, isPlaying: Bool, isMuted: Bool, positionMs: UInt64, rate: Float) {
    self.key = key
    self.isPlaying = isPlaying
    self.isMuted = isMuted
    self.positionMs = positionMs
    self.rate = rate
  }
}
