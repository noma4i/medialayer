import MediaLayer

extension MediaAudioSession {
  public func volumeRaises() async -> AsyncStream<Void> {
    guard let reading = await readVolume() else { return AsyncStream { $0.finish() } }

    return AsyncStream { continuation in
      let task = Task {
        var previous = reading.initial
        for await current in reading.changes {
          if isVolumeRaise(previous: previous, current: current) { continuation.yield() }
          previous = current
        }
        continuation.finish()
      }
      continuation.onTermination = { _ in task.cancel() }
    }
  }
}
