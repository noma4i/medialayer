actor AudioSessionOwner {
  enum Step { case done, busy, failed }

  static let deactivateAttempts = 4
  static let deactivateRetry: Duration = .milliseconds(150)

  private let port: AudioSessionPort
  private let box: DemandBox
  private let releaseDelay: Duration
  private var category: SessionPlan?
  private var active: Bool? = false
  private var seenChanges = 0

  init(port: AudioSessionPort, box: DemandBox, releaseDelay: Duration) {
    self.port = port
    self.box = box
    self.releaseDelay = releaseDelay
  }

  func reconcile() async -> Bool {
    for _ in 0..<Self.deactivateAttempts {
      var want = box.target()
      if want == .idle && active != false && releaseDelay > .zero {
        try? await Task.sleep(for: releaseDelay)
        want = box.target()
      }
      switch apply(want) {
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
      category = nil
      active = nil
    }
    guard case .active(let plan) = want else { return release() }

    if plan != category {
      do {
        try port.setCategory(plan)
      } catch {
        MediaTrace.emit("audio", "category \(plan.trace) failed \(error)")
        return .failed
      }
      MediaTrace.emit("audio", "category \(plan.trace)")
      category = plan
    }
    if active != true {
      do {
        try port.setActive(true)
      } catch {
        MediaTrace.emit("audio", "activate failed \(error)")
        return .failed
      }
      MediaTrace.emit("audio", "active")
      active = true
    }
    return .done
  }

  private func release() -> Step {
    guard active != false else { return .done }
    do {
      try port.setActive(false)
    } catch {
      MediaTrace.emit("audio", "deactivate busy \(error)")
      active = nil
      category = nil
      return .busy
    }
    MediaTrace.emit("audio", "inactive")
    active = false
    return .done
  }
}
