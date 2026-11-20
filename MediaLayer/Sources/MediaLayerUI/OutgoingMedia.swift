import Foundation
import MediaLayer
import Observation

/// The state of one outgoing item, shared by every `OutgoingMedia` of its key. The phase is decided by the core (`outgoingNext`); this
/// class only feeds it events and runs what the view of the core asks for: the remote fetch, the re-ask while on screen, the custody.
@MainActor @Observable
final class OutgoingEntry {
  let key: String
  private(set) var state: OutgoingState
  private(set) var view: OutgoingView
  private(set) var localURL: URL?
  @ObservationIgnored private let runtime: OutgoingRuntime
  @ObservationIgnored private var screens = 0
  @ObservationIgnored private var fetchTask: Task<Void, Never>?
  @ObservationIgnored private var fetchingURL: String?
  @ObservationIgnored private var reaskTask: Task<Void, Never>?
  @ObservationIgnored private var holdsCustody = false
  @ObservationIgnored private var watchers: [UInt64: AsyncStream<OutgoingPhase>.Continuation] = [:]
  @ObservationIgnored private var nextWatcher: UInt64 = 0

  init(runtime: OutgoingRuntime, key: String, kind: OutgoingKind, local: URL?, blurHash: String?) {
    self.runtime = runtime
    self.key = key
    let kept = local.map { runtime.store.keep($0, for: key) ?? $0 } ?? runtime.store.stored(for: key)
    localURL = kept
    holdsCustody = kept != nil
    let declared = outgoingDeclare(key: key, kind: kind, hasLocal: kept != nil, blurHash: blurHash)
    state = declared
    view = outgoingView(state: declared)
    runtime.register(self)
  }

  var kind: OutgoingKind { state.inputs.kind }
  var phase: OutgoingPhase { state.phase }
  var blurHash: String? { state.inputs.blurHash }
  var remote: String? { state.inputs.remote }
  var hasScreen: Bool { screens > 0 }
  /// While true the remote has been decided ready, the local is still kept for the frame that swaps it.
  private(set) var swapping = false

  func redeclare(local: URL?, blurHash: String?) {
    if let local, localURL == nil, phase != .ready {
      localURL = runtime.store.keep(local, for: key) ?? local
      holdsCustody = true
      state.inputs.hasLocal = true
    }
    if let blurHash, state.inputs.blurHash == nil { state.inputs.blurHash = blurHash }
    view = outgoingView(state: state)
  }

  func apply(_ event: OutgoingEvent) {
    let before = state.phase
    state = outgoingNext(previous: state, event: event)
    view = outgoingView(state: state)
    if state.phase != before { for watcher in watchers.values { watcher.yield(state.phase) } }
    settle()
  }

  /// The phase now and every change after it; a slow reader keeps the newest value only.
  func watchPhases() -> AsyncStream<OutgoingPhase> {
    let (stream, continuation) = AsyncStream<OutgoingPhase>.makeStream(bufferingPolicy: .bufferingNewest(1))
    nextWatcher += 1
    let id = nextWatcher
    watchers[id] = continuation
    continuation.onTermination = { [weak self] _ in Task { @MainActor in self?.watchers[id] = nil } }
    continuation.yield(state.phase)
    return stream
  }

  func appeared() {
    screens += 1
    settle()
  }

  func disappeared() {
    screens = max(0, screens - 1)
    settle()
  }

  /// The view drew the remote: the local kept for the swap frame can go.
  func swapDrawn() {
    guard swapping else { return }
    swapping = false
    releaseCustody()
  }

  func stop() {
    fetchTask?.cancel()
    fetchTask = nil
    reaskTask?.cancel()
    reaskTask = nil
  }

  private func releaseCustody() {
    guard holdsCustody else { return }
    holdsCustody = false
    runtime.store.release(key)
    localURL = nil
  }

  private func settle() {
    if state.phase == .ready {
      stop()
      if screens > 0, localURL != nil { swapping = true } else { releaseCustody() }
      runtime.finished(self)
      return
    }
    guard kind == .image, let url = state.inputs.remote else { return }
    if case .failed = state.phase {
      reaskTask?.cancel()
      reaskTask = nil
    }
    if fetchTask != nil, fetchingURL != url {
      fetchTask?.cancel()
      fetchTask = nil
    }
    if fetchTask == nil, state.inputs.remoteResult == nil {
      startFetch(url)
      return
    }
    if case .notReady = state.inputs.remoteResult, fetchTask == nil, let wait = view.reaskAfterMs, screens > 0 {
      if reaskTask == nil { startReask(url, after: wait) }
    } else {
      reaskTask?.cancel()
      reaskTask = nil
    }
  }

  private func startReask(_ url: String, after milliseconds: UInt32) {
    reaskTask = Task { [weak self] in
      guard let clock = self?.runtime.clock else { return }
      do { try await clock.sleep(milliseconds: milliseconds) } catch { return }
      guard let self, !Task.isCancelled else { return }
      reaskTask = nil
      guard state.inputs.remote == url, screens > 0 else { return }
      startFetch(url)
    }
  }

  /// One fetch of the remote at a time per item, at most `outgoingFetchSlots()` over all items; it goes on with no view on screen.
  private func startFetch(_ url: String) {
    fetchingURL = url
    fetchTask = Task { [weak self] in
      guard let runtime = self?.runtime else { return }
      guard await runtime.slots.acquire() else { return }
      let result = await runtime.remote.fetch(url: url, headers: [:])
      runtime.slots.release()
      guard let self, !Task.isCancelled, state.inputs.remote == url else { return }
      fetchTask = nil
      fetchingURL = nil
      apply(.remoteResult(result: result))
    }
  }
}

/// One outgoing media item of a chat: the original on the device, then the server copy, with the states between decided by the library.
/// Handles of one key share one state; the state is observable outside any view.
@MainActor
public final class OutgoingMedia {
  let entry: OutgoingEntry
  /// The app's own rule for opening the item; the default opens only in `.ready`.
  public var openable: ((OutgoingPhase) -> Bool)?

  public init(key: String, kind: OutgoingKind = .image, local: URL? = nil, blurHash: String? = nil) {
    let runtime = OutgoingRuntime.shared
    if let existing = runtime.entry(for: key) {
      existing.redeclare(local: local, blurHash: blurHash)
      entry = existing
    } else {
      entry = OutgoingEntry(runtime: runtime, key: key, kind: kind, local: local, blurHash: blurHash)
    }
  }

  init(entry: OutgoingEntry) { self.entry = entry }

  public var key: String { entry.key }
  public var kind: OutgoingKind { entry.kind }
  public var phase: OutgoingPhase { entry.phase }
  /// The phase now and every change after it, outside any view.
  public var phases: AsyncStream<OutgoingPhase> { entry.watchPhases() }
  public var blurHash: String? { entry.blurHash }
  public var canOpen: Bool { openable?(entry.phase) ?? entry.view.canOpen }
  /// The kept original, nil without a local or after the swap.
  public var local: URL? { entry.localURL }
  public var hasLocal: Bool { entry.state.inputs.hasLocal }
  /// The server url, once DBlayer has it. Setting it also means the upload is done.
  public var remote: String? {
    get { entry.remote }
    set { entry.apply(.remote(remote: newValue)) }
  }

  /// Folds DBlayer's upload states into the item until the stream ends. A repeated or older value changes nothing.
  public func forward<S: AsyncSequence>(uploads: S) async where S.Element == OutgoingUpload {
    do {
      for try await value in uploads { entry.apply(.upload(upload: value)) }
    } catch {}
  }

  /// Folds one upload state of DBlayer into the item.
  public func forward(upload: OutgoingUpload) { entry.apply(.upload(upload: upload)) }

  /// Fetches the remote again after `.failed(.remote)`. Anything else is ignored; an upload is sent again by DBlayer.
  public func retry() { entry.apply(.retry) }
}

extension Media {
  /// The item declared under `key`, from anywhere, or nil when none was declared in this process.
  @MainActor
  public static func outgoing(key: String) -> OutgoingMedia? {
    OutgoingRuntime.shared.entry(for: key).map { OutgoingMedia(entry: $0) }
  }
}

extension OutgoingPhase {
  /// The name drawn into the accessibility value and read by tests.
  public var name: String {
    switch self {
    case .sending: "sending"
    case .awaitingRemote: "awaitingRemote"
    case .fetchingRemote: "fetchingRemote"
    case .ready: "ready"
    case .failed(let reason):
      switch reason {
      case .upload: "failed.upload"
      case .remote: "failed.remote"
      }
    }
  }

  public var progress: Double {
    switch self {
    case .sending(let progress), .fetchingRemote(let progress): progress
    case .ready: 1
    case .awaitingRemote, .failed: 0
    }
  }

  public var isBusy: Bool {
    switch self {
    case .sending, .awaitingRemote, .fetchingRemote: true
    case .ready, .failed: false
    }
  }
}
