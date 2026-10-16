public struct MediaVolumeReading: Sendable {
  public let initial: Float
  public let changes: AsyncStream<Float>
}
