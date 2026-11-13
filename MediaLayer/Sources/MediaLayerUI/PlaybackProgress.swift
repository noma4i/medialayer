import AVFoundation
import Foundation
import MediaLayer

@MainActor
final class PlaybackProgress {
  private final class Demand {
    weak var slot: AVQueuePlayerSlot?
    var player: AVPlayer?
    var observer: Any?
    var consumers: [UUID: AsyncStream<Double>.Continuation] = [:]
    var reporters: [UUID: AsyncStream<PlayheadReport>.Continuation] = [:]
    var last: Double?

    func stopObservingTime() {
      if let observer { player?.removeTimeObserver(observer) }
      observer = nil
    }

    func stop() {
      stopObservingTime()
      player = nil
      slot = nil
    }

    deinit {
      if let player, let observer {
        DispatchQueue.main.async { player.removeTimeObserver(observer) }
      }
    }
  }

  private unowned let slots: LoadedSlots
  private var demands: [String: Demand] = [:]
  private(set) var samples = 0
  var producerCount: Int { demands.values.filter { $0.slot != nil }.count }
  var periodicObserverCount: Int { demands.values.filter { $0.observer != nil }.count }

  init(slots: LoadedSlots) { self.slots = slots }

  func stream(for key: String) -> AsyncStream<Double> {
    let id = UUID()
    let pair = AsyncStream<Double>.makeStream(bufferingPolicy: .bufferingNewest(1))
    let demand = demands[key] ?? Demand()
    demands[key] = demand
    demand.consumers[id] = pair.continuation
    if let last = demand.last { pair.continuation.yield(last) }
    pair.continuation.onTermination = { [weak self] _ in
      Task { @MainActor in self?.remove(id, key: key) }
    }
    changed(key)
    return pair.stream
  }

  func reports(for key: String) -> AsyncStream<PlayheadReport> {
    let id = UUID()
    let pair = AsyncStream<PlayheadReport>.makeStream(bufferingPolicy: .bufferingNewest(1))
    let demand = demands[key] ?? Demand()
    demands[key] = demand
    demand.reporters[id] = pair.continuation
    pair.continuation.onTermination = { [weak self] _ in
      Task { @MainActor in self?.remove(id, key: key) }
    }
    changed(key)
    return pair.stream
  }

  func changed(_ key: String) {
    guard let demand = demands[key] else { return }
    let slot = slots.slot(for: key)
    if demand.slot !== slot {
      demand.stop()
      demand.last = nil
      demand.slot = slot
      demand.player = slot?.player
    }
    if slot?.player.timeControlStatus != .playing {
      demand.stopObservingTime()
    } else if demand.observer == nil {
      demand.observer = slot?.player.addPeriodicTimeObserver(forInterval: CMTime(value: 1, timescale: 10), queue: .main) { [weak self] _ in
        MainActor.assumeIsolated { self?.emit(key) }
      }
    }
    emit(key)
  }

  private func emit(_ key: String) {
    guard let demand = demands[key], let slot = demand.slot else { return }
    samples += 1
    let sample = slot.sample()
    if !demand.reporters.isEmpty {
      let report = PlayheadReport(
        positionMs: sample.positionMs,
        durationMs: sample.durationMs ?? 0,
        atMs: UInt64(Date().timeIntervalSince1970 * 1000),
        rate: Double(slot.player.rate),
        playing: slot.player.timeControlStatus == .playing,
        jump: false,
        settling: slot.isSeeking
      )
      for reporter in demand.reporters.values { reporter.yield(report) }
    }
    guard let value = VideoProgress.fraction(positionMs: sample.positionMs, durationMs: sample.durationMs), demand.last != value else { return }
    demand.last = value
    for consumer in demand.consumers.values { consumer.yield(value) }
  }

  private func remove(_ id: UUID, key: String) {
    guard let demand = demands[key] else { return }
    demand.consumers[id] = nil
    demand.reporters[id] = nil
    if demand.consumers.isEmpty && demand.reporters.isEmpty {
      demand.stop()
      demands[key] = nil
    }
  }
}
