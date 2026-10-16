import Foundation

public final class MediaAudioSession: Sendable {
  public let focus: MediaAudioFocus
  private let box: DemandBox
  private let owner: AudioSessionOwner

  public convenience init(focus: MediaAudioFocus) {
    self.init(focus: focus, port: SystemAudioSession())
  }

  init(focus: MediaAudioFocus, port: AudioSessionPort, releaseDelay: Duration = .milliseconds(300)) {
    self.focus = focus
    box = DemandBox(focus: focus)
    owner = AudioSessionOwner(port: port, box: box, releaseDelay: releaseDelay)
  }

  public func activateRecording() async -> Bool {
    _ = box.update { $0.setRecording(true) }
    return await owner.reconcile()
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
}
