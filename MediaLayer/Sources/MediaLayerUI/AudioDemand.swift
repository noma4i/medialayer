struct AudioDemand {
  private(set) var audible: Set<UInt32> = []
  private(set) var recording = false

  mutating func setAudible(slot: UInt32, _ value: Bool) -> Bool {
    value ? audible.insert(slot).inserted : audible.remove(slot) != nil
  }

  mutating func setRecording(_ value: Bool) -> Bool {
    let changed = recording != value
    recording = value
    return changed
  }

  func target(for focus: MediaAudioFocus) -> SessionTarget {
    if recording { return .active(.recording) }

    return audible.isEmpty ? .idle : .active(.playback(focus))
  }
}
