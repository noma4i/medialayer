protocol AudioSessionPort: Sendable {
  func setCategory(_ plan: SessionPlan) throws
  func setActive(_ active: Bool) throws
}
