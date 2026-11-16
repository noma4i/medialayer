actor AudioSessionOwner {
  enum Step { case done, busy, failed }
  static let deactivateAttempts = 4
  static let deactivateRetry: Duration = .milliseconds(150)
  private let port: AudioSessionPort
  private let box: DemandBox
  private let releaseDelay: Duration
  private var needsRelease = false
  private var seenChanges = 0
  private var requested = 0
  private var running = false
  private var waiters: [(Int, CheckedContinuation<Bool, Never>)] = []

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

  func settle() async -> Bool {
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

  private func applyLatest() async -> Bool {
    for _ in 0..<Self.deactivateAttempts {
      if box.target() == .idle && needsRelease && releaseDelay > .zero {
        try? await Task.sleep(for: releaseDelay)
      }
      switch apply(box.target()) {
      case .done: return true
      case .failed: return false
      case .busy: try? await Task.sleep(for: Self.deactivateRetry)
      }
    }
    return false
  }

  private func apply(_ want: SessionTarget) -> Step {
    let changes = port.systemChanges
    if changes != seenChanges {
      seenChanges = changes
      needsRelease = true
    }
    guard case .active(let plan) = want else { return release() }
    if plan != port.currentPlan() {
      do { try port.setCategory(plan) } catch {
        MediaTrace.emit("audio", "category \(plan.trace) failed \(error)")
        return .failed
      }
      MediaTrace.emit("audio", "category \(plan.trace)")
    }
    do { try port.setActive(true) } catch {
      needsRelease = true
      MediaTrace.emit("audio", "activate failed \(error)")
      return .failed
    }
    MediaTrace.emit("audio", "active")
    needsRelease = true
    return .done
  }

  private func release() -> Step {
    guard needsRelease else { return .done }
    do { try port.setActive(false) } catch {
      MediaTrace.emit("audio", "deactivate busy \(error)")
      return .busy
    }
    MediaTrace.emit("audio", "inactive")
    needsRelease = false
    return .done
  }
}
