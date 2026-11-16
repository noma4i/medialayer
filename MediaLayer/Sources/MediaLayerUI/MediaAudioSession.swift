import Foundation

public final class MediaAudioSession: @unchecked Sendable {
  private let lock = NSLock()
  private var link: AudioLink?

  func bind(_ link: AudioLink?) { lock.withLock { self.link = link } }
  func setAudible(slot: UInt32, _ on: Bool) { lock.withLock { link }?.audible(slot: UInt64(slot), on) }

  public func readVolume() async -> MediaVolumeReading? {
    guard let link = lock.withLock({ link }) else { return nil }
    return await link.readVolume()
  }
}
