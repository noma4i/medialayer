import Foundation
import MediaLayer

extension RecordingRefusal: Error {}

public enum MediaCallKind: Sendable {
  case callKit(UUID)
  case unmanaged

  var core: CallKind {
    switch self {
    case let .callKit(id): .callKit(uuid: id.uuidString)
    case .unmanaged: .unmanaged
    }
  }
}

final class AudioLink: @unchecked Sendable {
  private let inputs: AudioInputs
  private let ports: AudioPorts
  private let inbox = DispatchQueue(label: "MediaLayer.audio.inbox")
  private let sessionQueue = DispatchQueue(label: "MediaLayer.audio.session")
  private var sessionConfigured = false
  private var next: UInt64 = 0
  private var closed = false
  private var pending: [UInt64: CheckedContinuation<Result<RecordingTicket, Error>, Never>] = [:]
  private var tickets: [UInt64: WeakRecordingTicket] = [:]
  private var calls: [UInt64: AsyncStream<CallState>.Continuation] = [:]
  private var unmanagedSpeakers: [UInt64: Bool] = [:]
  private var availabilitySinks: [UUID: AsyncStream<MicrophoneAvailability>.Continuation] = [:]
  private var currentAvailability: MicrophoneAvailability = .available
  private var tasks: [Task<Void, Never>] = []
  private let setVolumeMultiplier: @Sendable (Float) -> Void

  init(core: MediaLayer, ports: AudioPorts, setVolumeMultiplier: @escaping @Sendable (Float) -> Void = { _ in }) {
    self.inputs = core.audioInputs()
    self.ports = ports
    self.setVolumeMultiplier = setVolumeMultiplier
    let systemEvents = ports.session.systemEvents()
    let observedCalls = ports.calls.calls()
    tasks = [
      Task { [weak self] in for await event in systemEvents { self?.systemEvent(event) } },
      Task { [weak self] in for await calls in observedCalls { self?.send(.observedCalls(uuids: calls.sorted())) } }
    ]
    send(.permission(granted: ports.permission.granted()))
  }

  var availability: AsyncStream<MicrophoneAvailability> {
    let id = UUID()
    return AsyncStream { continuation in
      inbox.async {
        guard !self.closed else { continuation.finish(); return }
        self.availabilitySinks[id] = continuation
        continuation.yield(self.currentAvailability)
      }
      continuation.onTermination = { [weak self] _ in self?.inbox.async { self?.availabilitySinks[id] = nil } }
    }
  }

  func admit(_ kind: RecordingKind) async -> Result<RecordingTicket, Error> {
    await withCheckedContinuation { continuation in
      inbox.async {
        guard !self.closed else { continuation.resume(returning: .failure(MediaError.Closed)); return }
        let id = self.allocate()
        self.pending[id] = continuation
        self.process(.capture(id: id, kind: kind))
      }
    }
  }

  func call(_ kind: MediaCallKind) async -> MediaCallAudio {
    await withCheckedContinuation { continuation in
      inbox.async {
        let id = self.allocate()
        let (stream, sink) = AsyncStream<CallState>.makeStream()
        self.calls[id] = sink
        if case .unmanaged = kind { self.unmanagedSpeakers[id] = false }
        let call = MediaCallAudio(id: id, states: stream, link: self)
        self.process(.callStarted(id: id, kind: kind.core, speaker: false))
        continuation.resume(returning: call)
      }
    }
  }

  func audible(slot: UInt64, _ on: Bool) { send(.audible(slot: slot, on: on)) }
  func readVolume() async -> MediaVolumeReading {
    await withCheckedContinuation { continuation in
      inbox.async {
        let id = self.allocate()
        self.process(.volumeReadStarted(id: id))
        self.sessionQueue.async {
          let initial = self.ports.session.outputVolume()
          let changes = self.ports.session.volumeChanges()
          let stream = self.volumeStream(changes, id: id)
          continuation.resume(returning: MediaVolumeReading(initial: initial, changes: stream))
        }
      }
    }
  }

  private func volumeStream(_ changes: AsyncStream<Float>, id: UInt64) -> AsyncStream<Float> {
    let end: @Sendable () -> Void = { [weak self] in self?.send(.volumeReadEnded(id: id)) }
    return AsyncStream { sink in
      let task = Task { for await volume in changes { sink.yield(volume) }; sink.finish() }
      sink.onTermination = { _ in
        task.cancel()
        end()
      }
    }
  }
  func foreground(_ on: Bool) {
    if on { send(.permission(granted: ports.permission.granted())) }
    send(.foreground(on: on))
  }

  func send(_ input: AudioInput) { inbox.async { self.process(input) } }

  func setSpeaker(id: UInt64, on: Bool) {
    inbox.async {
      if self.unmanagedSpeakers[id] != nil {
        self.unmanagedSpeakers[id] = on
      }
      self.process(.callSpeaker(id: id, speaker: on))
      if self.unmanagedSpeakers[id] != nil { self.overrideSpeaker(on) }
    }
  }

  func endCall(id: UInt64) {
    inbox.async {
      self.process(.callEnded(id: id))
      self.unmanagedSpeakers[id] = nil
    }
  }

  func endTicket(id: UInt64) {
    inbox.async {
      self.tickets[id] = nil
      self.process(.captureEnded(id: id))
    }
  }

  func close() {
    inbox.async {
      guard !self.closed else { return }
      self.closed = true
      self.tasks.forEach { $0.cancel() }
      self.tasks.removeAll()
      self.pending.values.forEach { $0.resume(returning: .failure(MediaError.Closed)) }
      self.pending.removeAll()
      self.tickets.values.forEach { $0.value?.revoke(.interrupted) }
      self.tickets.removeAll()
      self.calls.values.forEach { $0.finish() }
      self.calls.removeAll()
      self.availabilitySinks.values.forEach { $0.finish() }
      self.availabilitySinks.removeAll()
      self.sessionQueue.async {
        if self.sessionConfigured { try? self.ports.session.deactivate(); self.sessionConfigured = false }
      }
    }
  }

  private func allocate() -> UInt64 { next += 1; return next }

  private func process(_ input: AudioInput) {
    guard !closed else { return }
    let actions: [AudioAction]
    do {
      actions = try inputs.send(input: input)
    } catch MediaError.Closed {
      pending.values.forEach { $0.resume(returning: .failure(MediaError.Closed)) }
      pending.removeAll()
      return
    } catch {
      Media.recordOpenFailure(error)
      pending.values.forEach { $0.resume(returning: .failure(error)) }
      pending.removeAll()
      return
    }
    for action in actions {
      switch action {
      case let .apply(generation, want, activate):
        let plan = SessionPlan.of(want)
        sessionQueue.async {
          let result: AppliedResult
          do { try self.ports.session.apply(plan, activate: activate); self.sessionConfigured = true; result = .ok }
          catch { result = .failed(code: Int64((error as NSError).code)) }
          self.send(.applied(generation: generation, result: result))
        }
      case let .deactivate(generation):
        sessionQueue.async {
          let result: AppliedResult
          do { try self.ports.session.deactivate(); self.sessionConfigured = false; result = .ok }
          catch is AudioSessionBusy { result = .busy }
          catch { result = .failed(code: Int64((error as NSError).code)) }
          self.send(.applied(generation: generation, result: result))
        }
      case let .admit(id):
        let ticket = RecordingTicket(id: id, link: self)
        tickets[id] = WeakRecordingTicket(value: ticket)
        pending.removeValue(forKey: id)?.resume(returning: .success(ticket))
      case let .refuse(id, reason): pending.removeValue(forKey: id)?.resume(returning: .failure(reason))
      case let .revoke(id, reason): tickets.removeValue(forKey: id)?.value?.revoke(reason)
      case let .callState(id, state):
        calls[id]?.yield(state)
        if state == .ended { calls.removeValue(forKey: id)?.finish() }
      case let .availability(value):
        currentAvailability = value
        availabilitySinks.values.forEach { $0.yield(value) }
      case let .volumeMultiplier(value): setVolumeMultiplier(value)
      case .pauseAll, .forgetPaused, .callHold, .resumePaused: break
      }
    }
  }

  private func systemEvent(_ event: SystemAudioEvent) {
    switch event {
    case .interruptionBegan: send(.interruptionBegan)
    case let .interruptionEnded(shouldResume): send(.interruptionEnded(shouldResume: shouldResume))
    case let .resumption(shouldResume): send(.resumptionRecommendation(shouldResume: shouldResume))
    case .mediaServicesReset: send(.mediaReset)
    case let .routeChanged(reason, hadPrivateOutput):
      inbox.async {
        self.process(.route(reason: reason, hadPrivateOutput: hadPrivateOutput))
        if self.unmanagedSpeakers.values.contains(true) { self.overrideSpeaker(true) }
      }
    }
  }

  private func overrideSpeaker(_ on: Bool) {
    sessionQueue.async { try? self.ports.session.overrideSpeaker(on) }
  }
}

private struct WeakRecordingTicket {
  weak var value: RecordingTicket?
}

final class RecordingTicket: @unchecked Sendable {
  let revoked: AsyncStream<RecordingRefusal>
  private let sink: AsyncStream<RecordingRefusal>.Continuation
  private let id: UInt64
  private weak var link: AudioLink?
  private let lock = NSLock()
  private var ended = false
  private var revokedReason: RecordingRefusal?
  var isActive: Bool { lock.withLock { !ended && revokedReason == nil } }

  init(id: UInt64, link: AudioLink) {
    self.id = id
    self.link = link
    (revoked, sink) = AsyncStream.makeStream()
  }

  func revoke(_ reason: RecordingRefusal) {
    lock.withLock { revokedReason = reason }
    sink.yield(reason)
    sink.finish()
  }
  func end() {
    guard lock.withLock({ if ended { return false }; ended = true; return true }) else { return }
    sink.finish()
    link?.endTicket(id: id)
  }
  deinit { end() }
}

public final class MediaCallAudio: @unchecked Sendable {
  public let states: AsyncStream<CallState>
  private let id: UInt64
  private weak var link: AudioLink?
  private let lock = NSLock()
  private var ended = false

  init(id: UInt64, states: AsyncStream<CallState>, link: AudioLink) { self.id = id; self.states = states; self.link = link }
  public func setSpeaker(_ on: Bool) { link?.setSpeaker(id: id, on: on) }
  public func systemActivated() { link?.send(.callSystemActivated(id: id)) }
  public func systemDeactivated() { link?.send(.callSystemDeactivated(id: id)) }
  public func end() {
    guard lock.withLock({ if ended { return false }; ended = true; return true }) else { return }
    link?.endCall(id: id)
  }
  deinit { end() }
}
