import Foundation

public final class MediaRecordingHold: @unchecked Sendable {
  private let lock = NSLock()
  private var action: (@Sendable () -> Void)?

  init(_ action: @escaping @Sendable () -> Void) { self.action = action }

  public func release() {
    let action = lock.withLock { let action = self.action; self.action = nil; return action }
    action?()
  }

  deinit { release() }
}

final class CallAudioHold: @unchecked Sendable {
  private let id: UUID
  private let box: DemandBox
  private let owner: AudioSessionOwner
  private let microphone: MicrophoneStatus
  private let lock = NSLock()
  private var released = false

  init(id: UUID, box: DemandBox, owner: AudioSessionOwner, microphone: MicrophoneStatus) {
    self.id = id
    self.box = box
    self.owner = owner
    self.microphone = microphone
  }

  var liveChanges: AsyncStream<Bool> { box.changes(for: id) }

  func setSpeaker(_ on: Bool) {
    _ = box.update { $0.setSpeaker(id, on); return true }
    MediaTrace.emit("audio", "call speaker \(on)")
    Task { [owner] in await owner.request() }
  }

  func systemActivated() { systemChanged(true) }
  func systemDeactivated() { systemChanged(false) }

  private func systemChanged(_ live: Bool) {
    let exists = box.update { demand in
      guard demand.calls.contains(id) else { return false }
      demand.systemLive = live
      return true
    }
    if exists { box.setLive(live) }
    MediaTrace.emit("audio", "call system \(live ? "activated" : "deactivated")")
    Task { [owner] in await owner.request() }
  }

  func release() {
    guard lock.withLock({ if released { return false }; released = true; return true }) else { return }
    box.removeCall(id)
    microphone.removeCall(id)
    MediaTrace.emit("audio", "call release")
    Task { [owner] in await owner.request() }
  }

  deinit { release() }
}

public final class MediaAudioSession: Sendable {
  public let focus: MediaAudioFocus
  private let box: DemandBox
  private let owner: AudioSessionOwner
  private let port: AudioSessionPort
  private let events: Task<Void, Never>
  let microphone: MicrophoneStatus

  public convenience init(focus: MediaAudioFocus) {
    self.init(focus: focus, port: SystemAudioSession(), microphonePort: SystemMicrophone())
  }

  init(focus: MediaAudioFocus, port: AudioSessionPort, releaseDelay: Duration = .milliseconds(300), microphonePort: MicrophonePort = AvailableMicrophone()) {
    self.focus = focus
    self.port = port
    box = DemandBox(focus: focus)
    microphone = MicrophoneStatus(port: microphonePort, box: box)
    let owner = AudioSessionOwner(port: port, box: box, releaseDelay: releaseDelay)
    self.owner = owner
    let stream = port.systemEvents()
    events = Task { for await _ in stream { await owner.request() } }
  }

  deinit { events.cancel() }

  public var isRecording: Bool {
    box.isRecording
  }

  public var microphoneAvailability: AsyncStream<MicrophoneAvailability> { microphone.changes }

  public func holdRecording() async -> MediaRecordingHold? {
    let id = UUID()
    guard box.update({ $0.microphoneAvailability == .available && $0.setRecording(id, true) }) else { return nil }
    MediaTrace.emit("audio", "recording hold holders=\(box.recordings)")
    let hold = MediaRecordingHold { [box, owner] in
      _ = box.update { $0.setRecording(id, false) }
      MediaTrace.emit("audio", "recording release holders=\(box.recordings)")
      Task { await owner.request() }
    }
    guard await owner.settle() else {
      microphone.refused()
      hold.release()
      _ = await owner.settle()
      return nil
    }
    return hold
  }

  func holdCall(managedBySystem: Bool, callUUID: UUID? = nil) async -> CallAudioHold? {
    let id = UUID()
    microphone.addCall(id, uuid: callUUID)
    _ = box.update { $0.addCall(id, managed: managedBySystem); return true }
    if managedBySystem { box.setLive(false) }
    MediaTrace.emit("audio", "call hold managed=\(managedBySystem)")
    let hold = CallAudioHold(id: id, box: box, owner: owner, microphone: microphone)
    guard await owner.settle() else {
      hold.release()
      _ = await owner.settle()
      return nil
    }
    return hold
  }

  public func readVolume() async -> MediaVolumeReading? {
    _ = box.update { $0.readVolume(1); return true }
    guard await owner.settle() else {
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

  func setAudible(slot: UInt32, _ value: Bool) {
    guard box.update({ $0.setAudible(slot: slot, value) }) else { return }

    MediaTrace.emit("audio", "audible slot=\(slot) \(value)")
    Task { [owner] in await owner.request() }
  }

  func settled() async -> Bool {
    await owner.settle()
  }

  private func release() {
    _ = box.update { $0.readVolume(-1); return true }
    Task { [owner] in await owner.request() }
  }
}
