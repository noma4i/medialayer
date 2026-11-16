import Foundation
#if os(iOS)
@preconcurrency import AVFoundation
@preconcurrency import CallKit

final class SystemMicrophone: NSObject, MicrophonePort, CXCallObserverDelegate, @unchecked Sendable {
  private let lock = NSLock()
  private let observer = CXCallObserver()
  private let queue = DispatchQueue(label: "com.noma4i.medialayer.microphone")
  private var calls: Set<UUID> = []
  private var interrupted = false
  private var tokens: [NSObjectProtocol] = []
  private var sinks: [UUID: AsyncStream<String>.Continuation] = [:]
  private var eventSinks: [UUID: AsyncStream<MicrophoneEvent>.Continuation] = [:]

  override init() {
    super.init()
    calls = Set(observer.calls.filter { !$0.hasEnded }.map(\.uuid))
    observer.setDelegate(self, queue: queue)
    let center = NotificationCenter.default
    tokens.append(center.addObserver(forName: AVAudioSession.routeChangeNotification, object: nil, queue: nil) { [weak self] _ in
      self?.queue.async { [weak self] in self?.emit("route") }
    })
    tokens.append(center.addObserver(forName: AVAudioSession.interruptionNotification, object: nil, queue: nil) { [weak self] note in
      guard let raw = note.userInfo?[AVAudioSessionInterruptionTypeKey] as? UInt,
            let type = AVAudioSession.InterruptionType(rawValue: raw) else { return }
      guard let self else { return }
      self.queue.async {
        self.lock.withLock { self.interrupted = type == .began && !self.calls.isEmpty }
        self.emit("interruption")
      }
    })
  }

  deinit { stop() }

  func stop() {
    let removed = lock.withLock { let result = tokens; tokens = []; return result }
    removed.forEach(NotificationCenter.default.removeObserver)
    observer.setDelegate(nil, queue: nil)
  }

  func current() -> MicrophoneState {
    MicrophoneState(
      calls: lock.withLock { calls },
      inputAvailable: AVAudioSession.sharedInstance().isInputAvailable,
      permissionGranted: AVAudioApplication.shared.recordPermission == .granted,
      interrupted: lock.withLock { interrupted }
    )
  }

  func changes() -> AsyncStream<String> {
    AsyncStream { continuation in
      let id = UUID()
      lock.withLock { sinks[id] = continuation }
      continuation.onTermination = { [weak self] _ in self?.lock.withLock { self?.sinks[id] = nil } }
    }
  }

  func events() -> AsyncStream<MicrophoneEvent> {
    AsyncStream { continuation in
      let id = UUID()
      lock.withLock { eventSinks[id] = continuation }
      continuation.onTermination = { [weak self] _ in self?.lock.withLock { self?.eventSinks[id] = nil } }
    }
  }

  func callObserver(_ callObserver: CXCallObserver, callChanged call: CXCall) {
    lock.withLock { if call.hasEnded { _ = calls.remove(call.uuid) } else { _ = calls.insert(call.uuid) } }
    emit("call")
  }

  private func emit(_ source: String) {
    lock.withLock { Array(sinks.values) }.forEach { $0.yield(source) }
    let event = MicrophoneEvent(state: current(), source: source)
    lock.withLock { Array(eventSinks.values) }.forEach { $0.yield(event) }
  }
}
#else
final class SystemMicrophone: MicrophonePort, @unchecked Sendable {
  func current() -> MicrophoneState { MicrophoneState() }
  func changes() -> AsyncStream<String> { AsyncStream { _ in } }
}
#endif
