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

public final class MediaAudioSession: Sendable {
  public let focus: MediaAudioFocus
  private let box: DemandBox
  private let owner: AudioSessionOwner
  private let port: AudioSessionPort
  private let events: Task<Void, Never>

  public convenience init(focus: MediaAudioFocus) {
    self.init(focus: focus, port: SystemAudioSession())
  }

  init(focus: MediaAudioFocus, port: AudioSessionPort, releaseDelay: Duration = .milliseconds(300)) {
    self.focus = focus
    self.port = port
    box = DemandBox(focus: focus)
    let owner = AudioSessionOwner(port: port, box: box, releaseDelay: releaseDelay)
    self.owner = owner
    let stream = port.systemEvents()
    events = Task { for await _ in stream { await owner.request() } }
  }

  deinit { events.cancel() }

  public var isRecording: Bool {
    box.isRecording
  }

  public func holdRecording() async -> MediaRecordingHold? {
    let id = UUID()
    _ = box.update { $0.setRecording(id, true) }
    MediaTrace.emit("audio", "recording hold holders=\(box.recordings)")
    let hold = MediaRecordingHold { [box, owner] in
      _ = box.update { $0.setRecording(id, false) }
      MediaTrace.emit("audio", "recording release holders=\(box.recordings)")
      Task { await owner.request() }
    }
    guard await owner.settle() else {
      hold.release()
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
