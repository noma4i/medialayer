import Foundation

actor AudioSessionOwner {
  enum Outcome { case success, activation(Error), failure(Error) }
  enum Step { case done, busy, activation(Error), failed(Error) }
  static let deactivateAttempts = 4
  static let deactivateRetry: Duration = .milliseconds(150)
  private let port: AudioSessionPort
  private let box: DemandBox
  private let releaseDelay: Duration
  private var needsRelease = false
  private var seenChanges = 0
  private var requested = 0
  private var running = false
  private var waiters: [(Int, CheckedContinuation<Outcome, Never>)] = []

  init(port: AudioSessionPort, box: DemandBox, releaseDelay: Duration) {
    self.port = port
    self.box = box
    self.releaseDelay = releaseDelay
  }

  func request() {
    requested += 1
    if !running {
      running = true
      Task { await pump() }
    }
  }

  func systemEvent(_ event: SystemAudioEvent) {
    if case .interruptionBegan = event {
      _ = box.update { $0.systemLive = false; return true }
      box.setLive(false)
    }
    request()
  }

  func settle() async -> Bool {
    if case .success = await settleResult() { return true }
    return false
  }

  func settleResult() async -> Outcome {
    await withCheckedContinuation { continuation in
      request()
      waiters.append((requested, continuation))
    }
  }

  private func pump() async {
    while true {
      let pass = requested
      let result = await applyLatest()
      let ready = waiters.filter { $0.0 <= pass }
      waiters.removeAll { $0.0 <= pass }
      for (_, continuation) in ready { continuation.resume(returning: result) }
      if requested == pass {
        running = false
        return
      }
    }
  }

  private func applyLatest() async -> Outcome {
    for _ in 0..<Self.deactivateAttempts {
      if box.target() == .idle && needsRelease && releaseDelay > .zero {
        try? await Task.sleep(for: releaseDelay)
      }
      switch apply(box.target()) {
      case .done: return .success
      case .activation(let error): return .activation(error)
      case .failed(let error): return .failure(error)
      case .busy: try? await Task.sleep(for: Self.deactivateRetry)
      }
    }
    return .failure(NSError(domain: "MediaLayer.AudioSession", code: 1))
  }

  private func apply(_ want: SessionTarget) -> Step {
    let changes = port.systemChanges
    if changes != seenChanges {
      seenChanges = changes
      needsRelease = true
    }
    let plan: SessionPlan
    let managed: Bool
    let call: Bool
    switch want {
    case .idle: return release()
    case .active(let value): plan = value; managed = false; call = false
    case .call(let value, let system): plan = value; managed = system; call = true
    }
    if plan != port.currentPlan() {
      do { try port.setCategory(plan) } catch {
        MediaTrace.emit("audio", "category \(plan.trace) failed \(error)")
        return .failed(error)
      }
      MediaTrace.emit("audio", "category \(plan.trace)")
    }
    if managed { needsRelease = true; return .done }
    do {
      try port.setActive(true)
    } catch {
      needsRelease = true
      MediaTrace.emit("audio", "activate failed \(error)")
      return .activation(error)
    }
    MediaTrace.emit("audio", "active")
    needsRelease = true
    if call { box.setLive(true) }
    return .done
  }

  private func release() -> Step {
    guard needsRelease else { return .done }
    do {
      try port.setActive(false)
    } catch {
      MediaTrace.emit("audio", "deactivate busy \(error)")
      return .busy
    }
    MediaTrace.emit("audio", "inactive")
    needsRelease = false
    return .done
  }
}
