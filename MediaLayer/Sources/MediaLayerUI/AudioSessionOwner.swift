actor AudioSessionOwner {
  enum Step { case done, busy, failed }

  static let deactivateAttempts = 4
  static let deactivateRetry: Duration = .milliseconds(150)

  private let port: AudioSessionPort
  private let box: DemandBox
  private let releaseDelay: Duration
  private var category: SessionPlan?
  private var active = false

  init(port: AudioSessionPort, box: DemandBox, releaseDelay: Duration) {
    self.port = port
    self.box = box
    self.releaseDelay = releaseDelay
  }

  func reconcile() async -> Bool {
    for _ in 0..<Self.deactivateAttempts {
      var want = box.target()
      if want == .idle && active && releaseDelay > .zero {
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
    guard case .active(let plan) = want else { return release() }

    if plan != category {
      guard (try? port.setCategory(plan)) != nil else { return .failed }

      category = plan
    }
    if !active {
      guard (try? port.setActive(true)) != nil else { return .failed }

      active = true
    }
    return .done
  }

  private func release() -> Step {
    guard active else { return .done }
    guard (try? port.setActive(false)) != nil else { return .busy }

    active = false
    return .done
  }
}
