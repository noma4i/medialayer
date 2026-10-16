struct AudioDemand {
  private(set) var audible: Set<UInt32> = []
  private(set) var recording = false
  private(set) var volumeReaders = 0

  mutating func setAudible(slot: UInt32, _ value: Bool) -> Bool {
    value ? audible.insert(slot).inserted : audible.remove(slot) != nil
  }

  mutating func setRecording(_ value: Bool) -> Bool {
    let changed = recording != value
    recording = value
    return changed
  }

  mutating func readVolume(_ delta: Int) {
    volumeReaders = max(0, volumeReaders + delta)
  }

  func target(for focus: MediaAudioFocus) -> SessionTarget {
    if recording { return .active(.recording) }

    if !audible.isEmpty { return .active(.playback(focus)) }

    return volumeReaders > 0 ? .active(.playback(.mix)) : .idle
  }
}
