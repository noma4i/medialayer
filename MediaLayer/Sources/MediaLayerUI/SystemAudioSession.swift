@preconcurrency import AVFoundation

struct SystemAudioSession: AudioSessionPort {
  private static let changes = SystemChanges()

  var systemChanges: Int { Self.changes.count }

  func currentPlan() -> SessionPlan {
    let session = AVAudioSession.sharedInstance()
    return SessionPlan(category: session.category, mode: session.mode, options: session.categoryOptions)
  }

  func systemEvents() -> AsyncStream<SystemAudioEvent> {
    AsyncStream { continuation in
      let center = NotificationCenter.default
      let session = AVAudioSession.sharedInstance()
      let tokens = [AVAudioSession.interruptionNotification, AVAudioSession.mediaServicesWereResetNotification, AVAudioSession.routeChangeNotification].map { name in
        center.addObserver(forName: name, object: session, queue: nil) { note in
          if name == AVAudioSession.interruptionNotification {
            guard let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
                  let type = AVAudioSession.InterruptionType(rawValue: raw) else { return }
            continuation.yield(type == .began ? .interruptionBegan : .interruptionEnded)
          } else {
            continuation.yield(name == AVAudioSession.mediaServicesWereResetNotification ? .mediaServicesReset : .routeChanged)
          }
        }
      }
      continuation.onTermination = { _ in tokens.forEach { center.removeObserver($0) } }
    }
  }

  func setCategory(_ plan: SessionPlan) throws {
    try AVAudioSession.sharedInstance().setCategory(plan.category, mode: plan.mode, options: plan.options)
  }

  func setActive(_ active: Bool) throws {
    try AVAudioSession.sharedInstance().setActive(active, options: active ? [] : [.notifyOthersOnDeactivation])
  }

  func outputVolume() -> Float {
    AVAudioSession.sharedInstance().outputVolume
  }

  func volumeChanges() -> AsyncStream<Float> {
    AsyncStream { continuation in
      let token = VolumeObservation(AVAudioSession.sharedInstance().observe(\.outputVolume, options: [.new]) { _, change in
        if let volume = change.newValue { continuation.yield(volume) }
      })
      continuation.onTermination = { _ in token.invalidate() }
    }
  }
}

private final class VolumeObservation: @unchecked Sendable {
  private let token: NSKeyValueObservation

  init(_ token: NSKeyValueObservation) {
    self.token = token
  }

  func invalidate() {
    token.invalidate()
  }
}

private final class SystemChanges: @unchecked Sendable {
  private let lock = NSLock()
  private var value = 0
  private var tokens: [NSObjectProtocol] = []

  init() {
    let center = NotificationCenter.default
    let session = AVAudioSession.sharedInstance()
    tokens = [AVAudioSession.interruptionNotification, AVAudioSession.mediaServicesWereResetNotification].map { name in
      center.addObserver(forName: name, object: session, queue: nil) { [weak self] note in
        self?.advance()
        MediaTrace.emit("audio", "system \(name.rawValue) \(note.userInfo ?? [:])")
      }
    }
    if MediaTrace.isOn {
      tokens.append(center.addObserver(forName: AVAudioSession.routeChangeNotification, object: session, queue: nil) { note in
        MediaTrace.emit("audio", "system route \(note.userInfo ?? [:]) now \(session.category.rawValue) \(session.mode.rawValue)")
      })
    }
  }

  var count: Int { lock.withLock { value } }

  private func advance() {
    lock.withLock { value += 1 }
  }
}
