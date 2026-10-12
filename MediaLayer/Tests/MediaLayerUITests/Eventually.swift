import Foundation
import Testing

func eventually(_ condition: String, timeout: Duration = .seconds(5), _ holds: () async -> Bool) async {
  let clock = ContinuousClock()
  let deadline = clock.now + timeout
  while await !holds() {
    guard clock.now < deadline else {
      Issue.record("never became true within \(timeout): \(condition)")
      return
    }
    try? await Task.sleep(for: .milliseconds(10))
  }
}
