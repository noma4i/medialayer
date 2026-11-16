enum SessionTarget: Equatable {
  case idle
  case active(SessionPlan)
  case call(SessionPlan, managedBySystem: Bool)
}
