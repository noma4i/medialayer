import Foundation

public final class MediaAudioSession: Sendable {
  public let focus: MediaAudioFocus
  private let box: DemandBox
  private let owner: AudioSessionOwner
  private let port: AudioSessionPort

  public convenience init(focus: MediaAudioFocus) {
    self.init(focus: focus, port: SystemAudioSession())
  }

  init(focus: MediaAudioFocus, port: AudioSessionPort, releaseDelay: Duration = .milliseconds(300)) {
    self.focus = focus
    self.port = port
    box = DemandBox(focus: focus)
    owner = AudioSessionOwner(port: port, box: box, releaseDelay: releaseDelay)
  }

  public var isRecording: Bool {
    box.isRecording
  }

  public func activateRecording() async -> Bool {
    _ = box.update { $0.setRecording(true) }
    guard await owner.reconcile() else {
      _ = box.update { $0.setRecording(false) }
      return false
    }
    return true
  }

  public func readVolume() async -> MediaVolumeReading? {
    _ = box.update { $0.readVolume(1); return true }
    guard await owner.reconcile() else {
      release()
      return nil
    }

    let changes = port.volumeChanges()
    let (stream, continuation) = AsyncStream<Float>.makeStream()
    let forward = Task { for await volume in changes { continuation.yield(volume) } }
    continuation.onTermination = { [weak self] _ in
      forward.cancel()
      self?.release()
    }
    return MediaVolumeReading(initial: port.outputVolume(), changes: stream)
  }

  public func deactivateRecording() async {
    _ = box.update { $0.setRecording(false) }
    _ = await owner.reconcile()
  }

  func setAudible(slot: UInt32, _ value: Bool) {
    guard box.update({ $0.setAudible(slot: slot, value) }) else { return }

    Task { [owner] in _ = await owner.reconcile() }
  }

  func settled() async -> Bool {
    await owner.reconcile()
  }

  private func release() {
    _ = box.update { $0.readVolume(-1); return true }
    Task { [owner] in _ = await owner.reconcile() }
  }
}
