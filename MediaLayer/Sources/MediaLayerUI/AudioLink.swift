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

private enum PendingAudioAction {
  case core(AudioAction)
  case speaker(Bool)
}

private final class Admission: @unchecked Sendable {
  private let lock = NSLock()
  private var cancelled = false
  var id: UInt64?

  func cancel() { lock.withLock { cancelled = true } }
  var isCancelled: Bool { lock.withLock { cancelled } }
}

protocol AudioInputOwner: AnyObject, Sendable {
  func restore(_ input: AudioInputHandle, generation: UInt64)
  func stop(generation: UInt64)
}

final class AudioInputHandle: @unchecked Sendable {
  let id: UInt64
  private weak var link: AudioLink?

  init(id: UInt64, link: AudioLink) { self.id = id; self.link = link }
  deinit { close() }

  func sound(generation: UInt64, heard: Bool) { link?.send(.noSound(id: id, generation: generation, on: !heard)) }
  func restoreFailed(generation: UInt64) { link?.send(.restoreFailed(id: id, generation: generation)) }
  func close() { link?.closeInput(id: id) }
}

final class AudioLink: @unchecked Sendable {
  private let inputs: AudioInputs
  private let ports: AudioPorts
  private let inbox = DispatchQueue(label: "MediaLayer.audio.inbox")
  private let inboxKey = DispatchSpecificKey<Bool>()
  private let sessionQueue = DispatchQueue(label: "MediaLayer.audio.session")
  private var queuedActions: [PendingAudioAction] = []
  private var processing = false
  private var sessionConfigured = false
  private let idLock = NSLock()
  private var next: UInt64 = 0
  private var closed = false
  private var closeFinished = false
  private var closeWaiters: [@Sendable () -> Void] = []
  private var pending: [UInt64: CheckedContinuation<Result<RecordingTicket, Error>, Never>] = [:]
  private var tickets: [UInt64: WeakRecordingTicket] = [:]
  private var inputRegistry: [UInt64: InputRegistration] = [:]
  private var pendingStops: [RecordingTicket] = []
  private var calls: [UInt64: CallSignals] = [:]
  private var unmanagedSpeakers: [UInt64: Bool] = [:]
  private var callKitIDs: [UInt64: String] = [:]
  private var availabilitySinks: [UUID: AsyncStream<MicrophoneAvailability>.Continuation] = [:]
  private var currentAvailability: MicrophoneAvailability = .available
  private var stopEvents: [@Sendable () -> Void] = []
  private var closeActions: [@Sendable () -> Void] = []
  private let setVolumeMultiplier: @Sendable (Float) -> Void

  init(core: MediaLayer, ports: AudioPorts, setVolumeMultiplier: @escaping @Sendable (Float) -> Void = { _ in }) {
    self.inputs = core.audioInputs()
    self.ports = ports
    self.setVolumeMultiplier = setVolumeMultiplier
    inbox.setSpecific(key: inboxKey, value: true)
    stopEvents = [
      ports.session.systemEvents { [weak self] event in self?.systemEvent(event) },
      ports.calls.calls { [weak self] calls in self?.observedCalls(calls) },
      ports.permission.changes { [weak self] granted in self?.send(.permission(granted: granted)) }
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
    let admission = Admission()
    return await withTaskCancellationHandler {
      await withCheckedContinuation { continuation in
        inbox.async {
          guard !admission.isCancelled else { continuation.resume(returning: .failure(CancellationError())); return }
          guard !self.closed else { continuation.resume(returning: .failure(MediaError.Closed)); return }
          let id = self.allocate()
          admission.id = id
          self.process(.permission(granted: self.ports.permission.granted()))
          self.pending[id] = continuation
          self.process(.capture(id: id, kind: kind))
        }
      }
    } onCancel: {
      admission.cancel()
      self.inbox.async {
        guard let id = admission.id, let continuation = self.pending.removeValue(forKey: id) else { return }
        self.process(.captureEnded(id: id))
        continuation.resume(returning: .failure(CancellationError()))
      }
    }
  }

  func call(_ kind: MediaCallKind) async throws -> MediaCallAudio {
    try await withCheckedThrowingContinuation { continuation in
      inbox.async {
        guard !self.closed else { continuation.resume(throwing: MediaError.Closed); return }
        let id = self.allocate()
        let signals = CallSignals()
        self.calls[id] = signals
        if case .unmanaged = kind { self.unmanagedSpeakers[id] = false }
        if case let .callKit(uuid) = kind { self.callKitIDs[id] = uuid.uuidString }
        let call = MediaCallAudio(id: id, signals: signals, link: self)
        self.process(.callStarted(id: id, kind: kind.core, speaker: false))
        continuation.resume(returning: call)
      }
    }
  }

  func openInput(_ kind: RecordingKind, owner: AudioInputOwner) -> AudioInputHandle {
    let handle = AudioInputHandle(id: allocate(), link: self)
    inbox.async {
      guard !self.closed else { return }
      self.inputRegistry[handle.id] = InputRegistration(owner: owner, handle: handle)
      self.process(.inputOpened(id: handle.id, kind: kind))
    }
    return handle
  }

  func closeInput(id: UInt64) {
    inbox.async {
      self.inputRegistry.removeValue(forKey: id)?.stopLive()
      self.process(.inputClosed(id: id))
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
      self.onClose { task.cancel(); sink.finish() }
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

  func onClose(_ action: @escaping @Sendable () -> Void) {
    inbox.async { if self.closed { action() } else { self.closeActions.append(action) } }
  }

  func setSpeaker(id: UInt64, on: Bool) {
    inbox.async {
      if self.unmanagedSpeakers[id] != nil {
        self.unmanagedSpeakers[id] = on
      }
      self.process(.callSpeaker(id: id, speaker: on))
      if self.unmanagedSpeakers[id] != nil { self.enqueueSpeaker(on) }
    }
  }

  func endCall(id: UInt64) {
    let end = {
      self.process(.callEnded(id: id))
      self.unmanagedSpeakers[id] = nil
      self.callKitIDs[id] = nil
    }
    if DispatchQueue.getSpecific(key: inboxKey) != nil { end() }
    else { inbox.sync(execute: end) }
  }

  func endTicket(id: UInt64) {
    inbox.async {
      self.tickets[id] = nil
      self.process(.captureEnded(id: id))
    }
  }

  func close(completion: @escaping @Sendable () -> Void = {}) {
    inbox.async {
      if self.closed {
        if self.closeFinished { completion() } else { self.closeWaiters.append(completion) }
        return
      }
      self.closed = true
      self.closeWaiters.append(completion)
      self.stopEvents.forEach { $0() }
      self.stopEvents.removeAll()
      self.pending.values.forEach { $0.resume(returning: .failure(MediaError.Closed)) }
      self.pending.removeAll()
      let stops = self.tickets.values.compactMap(\.value)
      stops.forEach { $0.revoke(.interrupted) }
      self.tickets.removeAll()
      self.inputRegistry.values.forEach { $0.stopLive() }
      self.inputRegistry.removeAll()
      self.closeActions.forEach { $0() }
      self.closeActions.removeAll()
      self.calls.values.forEach { $0.end() }
      self.calls.removeAll()
      self.availabilitySinks.values.forEach { $0.finish() }
      self.availabilitySinks.removeAll()
      self.sessionQueue.async {
        stops.forEach { $0.waitForStop() }
        if self.sessionConfigured {
          do { try self.ports.session.deactivate(); self.sessionConfigured = false }
          catch { Media.recordNativeFailure(error, source: .session) }
        }
        self.inbox.async {
          self.closeFinished = true
          let waiters = self.closeWaiters
          self.closeWaiters.removeAll()
          waiters.forEach { $0() }
        }
      }
    }
  }

  func awaitClose() async {
    await withCheckedContinuation { continuation in close { continuation.resume() } }
  }

  private func allocate() -> UInt64 { idLock.withLock { next += 1; return next } }

  private func observedCalls(_ observed: Set<String>) {
    inbox.async {
      self.process(.observedCalls(uuids: observed.sorted()))
      let external = !observed.subtracting(self.callKitIDs.values).isEmpty
      self.process(.phoneCall(active: external))
    }
  }

  private func report(_ error: Error, source: FailureSource) {
    let native = error as NSError
    send(.failure(failure: NativeFailure(source: source, domain: native.domain, code: Int64(native.code), message: native.localizedDescription)))
  }

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
      Media.recordFailure(error)
      pending.values.forEach { $0.resume(returning: .failure(error)) }
      pending.removeAll()
      return
    }
    queuedActions.append(contentsOf: actions.map(PendingAudioAction.core))
    runPendingActions()
  }

  private func enqueueSpeaker(_ on: Bool) {
    queuedActions.append(.speaker(on))
    runPendingActions()
  }

  private func runPendingActions() {
    guard !processing else { return }
    processing = true
    runActions()
  }

  private func runActions() {
    guard !closed else { processing = false; return }
    while !queuedActions.isEmpty {
      let queued = queuedActions.removeFirst()
      guard case let .core(action) = queued else {
        if case let .speaker(on) = queued { overrideSpeaker(on) }
        continue
      }
      switch action {
      case let .awaitPaused(barrier):
        Task.detached {
          await self.inputs.paused(barrier: barrier)
          self.inbox.async { self.runActions() }
        }
        return
      case let .apply(generation, want, activate):
        let plan = SessionPlan.of(want)
        pendingStops.removeAll { $0.isEnded }
        let stops = pendingStops
        sessionQueue.async {
          stops.forEach { $0.waitForStop() }
          let result: AppliedResult
          do { try self.ports.session.apply(plan, activate: activate); self.sessionConfigured = true; result = .ok }
          catch { self.report(error, source: .session); result = .failed(code: Int64((error as NSError).code)) }
          self.send(.applied(generation: generation, result: result))
          if case .failed = result {
            self.inbox.async { for id in self.calls.keys { self.process(.callSystemDeactivated(id: id)) } }
          }
        }
      case let .deactivate(generation):
        pendingStops.removeAll { $0.isEnded }
        let stops = pendingStops
        sessionQueue.async {
          stops.forEach { $0.waitForStop() }
          let result: AppliedResult
          do { try self.ports.session.deactivate(); self.sessionConfigured = false; result = .ok }
          catch is AudioSessionBusy { result = .busy }
          catch { self.report(error, source: .session); result = .failed(code: Int64((error as NSError).code)) }
          self.send(.applied(generation: generation, result: result))
        }
      case let .admit(id):
        guard let continuation = pending.removeValue(forKey: id) else { continue }
        let ticket = RecordingTicket(id: id, link: self)
        tickets[id] = WeakRecordingTicket(value: ticket)
        continuation.resume(returning: .success(ticket))
      case let .refuse(id, reason): pending.removeValue(forKey: id)?.resume(returning: .failure(reason))
      case let .revoke(id, reason):
        if let ticket = tickets.removeValue(forKey: id)?.value {
          pendingStops.append(ticket)
          ticket.revoke(reason)
        }
      case let .callState(id, state):
        if state == .ended { calls.removeValue(forKey: id)?.end() }
        else { calls[id]?.setState(state) }
      case let .availability(value):
        currentAvailability = value
        availabilitySinks.values.forEach { $0.yield(value) }
      case let .restoreInput(id, _, generation):
        guard var input = inputRegistry[id], let owner = input.owner, let handle = input.handle else { continue }
        input.generation = generation
        inputRegistry[id] = input
        owner.restore(handle, generation: generation)
      case let .stopInput(id, generation):
        guard var input = inputRegistry[id] else { continue }
        input.generation = nil
        inputRegistry[id] = input
        input.owner?.stop(generation: generation)
      case let .volumeMultiplier(value): setVolumeMultiplier(value)
      case let .route(speaker): overrideSpeaker(speaker)
      case let .failure(failure): Media.publishFailure(failure)
      case .pauseAll, .forgetPaused, .callHold, .resumePaused: break
      }
    }
    processing = false
  }

  private func systemEvent(_ event: SystemAudioEvent) {
    switch event {
    case .interruptionBegan: send(.interruptionBegan)
    case let .interruptionEnded(shouldResume): send(.interruptionEnded(shouldResume: shouldResume))
    case let .resumption(shouldResume): send(.resumptionRecommendation(shouldResume: shouldResume))
    case .mediaServicesReset: send(.mediaReset)
    case let .routeChanged(reason, hadPrivateOutput):
      inbox.async {
        self.process(.route(reason: reason, lostPrivateOutput: hadPrivateOutput))
        if self.unmanagedSpeakers.values.contains(true) { self.enqueueSpeaker(true) }
      }
    }
  }

  private func overrideSpeaker(_ on: Bool) {
    sessionQueue.async {
      let route: CallRoute
      do { try self.ports.session.overrideSpeaker(on); route = on ? .speaker : .receiver }
      catch { self.report(error, source: .speaker); route = .failed }
      self.inbox.async { self.calls.values.forEach { $0.setRoute(route) } }
    }
  }
}

private struct WeakRecordingTicket {
  weak var value: RecordingTicket?
}

private struct InputRegistration {
  weak var owner: AudioInputOwner?
  weak var handle: AudioInputHandle?
  var generation: UInt64?

  init(owner: AudioInputOwner, handle: AudioInputHandle) { self.owner = owner; self.handle = handle }

  func stopLive() {
    guard let generation else { return }
    owner?.stop(generation: generation)
  }
}

private final class CallSignals: @unchecked Sendable {
  private let lock = NSLock()
  private var state: CallState = .connecting
  private var route: CallRoute = .receiver
  private var closed = false
  private let awake = ScreenAwakeHold(kind: .always)
  private var stateSinks: [UUID: AsyncStream<CallState>.Continuation] = [:]
  private var routeSinks: [UUID: AsyncStream<CallRoute>.Continuation] = [:]

  init() { awake.set(playing: true) }

  func release() { awake.set(playing: false) }

  func states() -> AsyncStream<CallState> {
    AsyncStream { sink in
      let id = UUID()
      let finished = lock.withLock { () -> Bool in
        sink.yield(state)
        if !closed { stateSinks[id] = sink }
        return closed
      }
      if finished { sink.finish() }
      sink.onTermination = { [weak self] _ in self?.lock.withLock { self?.stateSinks[id] = nil } }
    }
  }

  func routes() -> AsyncStream<CallRoute> {
    AsyncStream { sink in
      let id = UUID()
      let finished = lock.withLock { () -> Bool in
        sink.yield(route)
        if !closed { routeSinks[id] = sink }
        return closed
      }
      if finished { sink.finish() }
      sink.onTermination = { [weak self] _ in self?.lock.withLock { self?.routeSinks[id] = nil } }
    }
  }

  func setState(_ value: CallState) {
    lock.withLock {
      guard state != value else { return }
      state = value
      stateSinks.values.forEach { $0.yield(value) }
    }
  }

  func setRoute(_ value: CallRoute) {
    lock.withLock { route = value; routeSinks.values.forEach { $0.yield(value) } }
  }

  func end() {
    let sinks = lock.withLock { () -> ([AsyncStream<CallState>.Continuation], [AsyncStream<CallRoute>.Continuation]) in
      guard !closed else { return ([], []) }
      closed = true
      state = .ended
      let result = (Array(stateSinks.values), Array(routeSinks.values))
      stateSinks.removeAll()
      routeSinks.removeAll()
      return result
    }
    release()
    sinks.0.forEach { $0.yield(.ended); $0.finish() }
    sinks.1.forEach { $0.finish() }
  }
}

final class RecordingTicket: @unchecked Sendable {
  let revoked: AsyncStream<RecordingRefusal>
  private let sink: AsyncStream<RecordingRefusal>.Continuation
  private let id: UInt64
  private weak var link: AudioLink?
  private let lock = NSLock()
  private var ended = false
  private var revokedReason: RecordingRefusal?
  private var revokeObservers: [@Sendable (RecordingRefusal) -> Void] = []
  private let stopped = DispatchGroup()
  var isActive: Bool { lock.withLock { !ended && revokedReason == nil } }
  var isEnded: Bool { lock.withLock { ended } }

  init(id: UInt64, link: AudioLink) {
    self.id = id
    self.link = link
    stopped.enter()
    (revoked, sink) = AsyncStream.makeStream()
  }

  func revoke(_ reason: RecordingRefusal) {
    let observers = lock.withLock { () -> [@Sendable (RecordingRefusal) -> Void] in
      revokedReason = reason
      defer { revokeObservers = [] }
      return revokeObservers
    }
    observers.forEach { $0(reason) }
    sink.yield(reason)
    sink.finish()
  }

  func whenRevoked(_ action: @escaping @Sendable (RecordingRefusal) -> Void) {
    let reason = lock.withLock { () -> RecordingRefusal? in
      if let revokedReason { return revokedReason }
      revokeObservers.append(action)
      return nil
    }
    if let reason { action(reason) }
  }
  func start<T>(_ operation: () throws -> T) throws -> T {
    try lock.withLock {
      if let revokedReason { throw revokedReason }
      guard !ended else { throw RecordingRefusal.interrupted }
      return try operation()
    }
  }
  func end() {
    guard lock.withLock({ if ended { return false }; ended = true; return true }) else { return }
    stopped.leave()
    sink.finish()
    link?.endTicket(id: id)
  }
  func waitForStop() { stopped.wait() }
  deinit { end() }
}

public final class MediaCallAudio: @unchecked Sendable {
  public var states: AsyncStream<CallState> { signals.states() }
  public var route: AsyncStream<CallRoute> { signals.routes() }
  private let signals: CallSignals
  private let id: UInt64
  private weak var link: AudioLink?
  private let lock = NSLock()
  private var ended = false

  fileprivate init(id: UInt64, signals: CallSignals, link: AudioLink) { self.id = id; self.signals = signals; self.link = link }
  public func setSpeaker(_ on: Bool) { link?.setSpeaker(id: id, on: on) }
  public func systemActivated() { link?.send(.callSystemActivated(id: id)) }
  public func systemDeactivated() { link?.send(.callSystemDeactivated(id: id)) }
  public func end() {
    guard lock.withLock({ if ended { return false }; ended = true; return true }) else { return }
    signals.release()
    link?.endCall(id: id)
  }
  deinit { end() }
}
