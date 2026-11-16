@preconcurrency import AVFoundation
import Foundation
@testable import MediaLayerUI

final class FakeAudioSessionPort: AudioSessionPort, @unchecked Sendable {
  private let lock = NSLock()
  private var calls: [String] = []
  private var mainThreadCalls = 0
  private var busyDeactivations: Int
  private var volumeSink: AsyncStream<Float>.Continuation?
  private var failingCategories: Set<AVAudioSession.Category>
  private var currentCategory: AVAudioSession.Category?
  private var plan = SessionPlan(category: .soloAmbient, mode: .default, options: [])
  private var eventSink: AsyncStream<SystemAudioEvent>.Continuation?
  private var volumeEnded = false
  private var changes = 0

  init(busyDeactivations: Int = 0, failingCategories: Set<AVAudioSession.Category> = []) {
    self.busyDeactivations = busyDeactivations
    self.failingCategories = failingCategories
  }

  static func category(_ plan: SessionPlan) -> String {
    "category \(plan.category.rawValue) \(plan.mode.rawValue) \(plan.options.rawValue)"
  }

  var log: [String] { lock.withLock { calls } }
  var volumeStreamEnded: Bool { lock.withLock { volumeEnded } }
  var onMain: Int { lock.withLock { mainThreadCalls } }

  func setCategory(_ plan: SessionPlan) throws {
    lock.withLock { currentCategory = plan.category; self.plan = plan }
    record(Self.category(plan))
  }

  func setActive(_ active: Bool) throws {
    defer { TestSignal.shared.fire() }
    try lock.withLock {
      if active, let current = currentCategory, failingCategories.contains(current) {
        calls.append("active true FAILED")
        throw NSError(domain: "fake", code: 2)
      }
      if !active && busyDeactivations > 0 {
        busyDeactivations -= 1
        calls.append("active false BUSY")
        throw NSError(domain: "fake", code: 1)
      }
    }
    record("active \(active)")
  }

  var systemChanges: Int { lock.withLock { changes } }

  func currentPlan() -> SessionPlan { lock.withLock { plan } }

  func systemEvents() -> AsyncStream<SystemAudioEvent> {
    let (stream, continuation) = AsyncStream<SystemAudioEvent>.makeStream()
    lock.withLock { eventSink = continuation }
    return stream
  }

  func systemDeactivates() {
    lock.withLock { changes += 1 }
    record("system deactivated")
  }

  func outputVolume() -> Float { 0.5 }

  func volumeChanges() -> AsyncStream<Float> {
    let (stream, continuation) = AsyncStream<Float>.makeStream()
    lock.withLock { volumeSink = continuation }
    continuation.onTermination = { [self] _ in lock.withLock { volumeEnded = true }; TestSignal.shared.fire() }
    return stream
  }

  func stopFailing(_ category: AVAudioSession.Category) {
    lock.withLock { _ = failingCategories.remove(category) }
  }

  func pressVolume(_ value: Float) {
    lock.withLock { volumeSink }?.yield(value)
  }

  private func record(_ call: String) {
    lock.withLock {
      calls.append(call)
      if Thread.isMainThread { mainThreadCalls += 1 }
    }
    TestSignal.shared.fire()
  }
}
