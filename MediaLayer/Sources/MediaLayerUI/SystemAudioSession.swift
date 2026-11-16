@preconcurrency import AVFoundation
@preconcurrency import CallKit
import Foundation
import MediaLayer

struct SystemAudioSession: AudioSessionPort {
  func systemEvents() -> AsyncStream<SystemAudioEvent> {
    AsyncStream { continuation in
      let center = NotificationCenter.default
      let session = AVAudioSession.sharedInstance()
      var tokens: [NSObjectProtocol] = []
      tokens.append(center.addObserver(forName: AVAudioSession.interruptionNotification, object: session, queue: nil) { note in
        guard let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
              let type = AVAudioSession.InterruptionType(rawValue: raw) else { return }
        if type == .began {
          continuation.yield(.interruptionBegan)
        } else {
          let rawOptions = note.userInfo?[AVAudioSessionInterruptionOptionKey] as? UInt ?? 0
          continuation.yield(.interruptionEnded(shouldResume: AVAudioSession.InterruptionOptions(rawValue: rawOptions).contains(.shouldResume)))
        }
      })
      tokens.append(center.addObserver(forName: AVAudioSession.mediaServicesWereResetNotification, object: session, queue: nil) { _ in
        continuation.yield(.mediaServicesReset)
      })
      tokens.append(center.addObserver(forName: AVAudioSession.routeChangeNotification, object: session, queue: nil) { note in
        let raw = note.userInfo?[AVAudioSessionRouteChangeReasonKey] as? UInt ?? 0
        let previous = (note.userInfo?[AVAudioSessionRouteChangePreviousRouteKey] as? AVAudioSessionRouteDescription)?.outputs.map(\.portType) ?? []
        let reason: RouteReason
        switch AVAudioSession.RouteChangeReason(rawValue: raw) {
        case .oldDeviceUnavailable: reason = .oldDeviceUnavailable
        case .newDeviceAvailable: reason = .newDeviceAvailable
        case .categoryChange: reason = .categoryChange
        case .override: reason = .override
        default: reason = .other
        }
        continuation.yield(.routeChanged(reason, hadPrivateOutput: hadPrivateOutput(previous)))
      })
      if #available(iOS 27.0, *) {
        tokens.append(center.addObserver(forName: AVAudioSession.resumptionRecommendationNotification, object: session, queue: nil) { _ in
          continuation.yield(.resumption(shouldResume: session.recommendation == .shouldResume))
        })
      }
      let registered = tokens
      continuation.onTermination = { _ in registered.forEach { center.removeObserver($0) } }
    }
  }

  func apply(_ plan: SessionPlan, activate: Bool) throws {
    let session = AVAudioSession.sharedInstance()
    try session.setCategory(plan.category, mode: plan.mode, options: plan.options)
    if activate { try session.setActive(true) }
  }

  func deactivate() throws {
    do {
      try AVAudioSession.sharedInstance().setActive(false, options: [.notifyOthersOnDeactivation])
    } catch {
      let value = error as NSError
      if value.domain == NSOSStatusErrorDomain && value.code == AVAudioSession.ErrorCode.isBusy.rawValue { throw AudioSessionBusy() }
      throw error
    }
  }

  func overrideSpeaker(_ on: Bool) throws {
    try AVAudioSession.sharedInstance().overrideOutputAudioPort(on ? .speaker : .none)
  }

  func outputVolume() -> Float { AVAudioSession.sharedInstance().outputVolume }

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
  init(_ token: NSKeyValueObservation) { self.token = token }
  func invalidate() { token.invalidate() }
}

struct SystemMicrophonePermission: MicrophonePermissionPort {
  func granted() -> Bool { AVAudioApplication.shared.recordPermission == .granted }
}

final class SystemCallObserver: NSObject, CallObserverPort, CXCallObserverDelegate, @unchecked Sendable {
  private let observer = CXCallObserver()
  private let lock = NSLock()
  private var sink: AsyncStream<Set<String>>.Continuation?

  override init() {
    super.init()
    observer.setDelegate(self, queue: DispatchQueue(label: "MediaLayer.calls"))
  }

  func calls() -> AsyncStream<Set<String>> {
    AsyncStream { continuation in
      lock.withLock { sink = continuation }
      publish()
    }
  }

  func callObserver(_ callObserver: CXCallObserver, callChanged call: CXCall) { publish() }

  private func publish() {
    let active = Set(observer.calls.filter { !$0.hasEnded }.map { $0.uuid.uuidString })
    lock.withLock { sink }?.yield(active)
  }
}
