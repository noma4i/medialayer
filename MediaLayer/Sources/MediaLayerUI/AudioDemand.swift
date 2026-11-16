import Foundation

struct AudioDemand {
  private(set) var audible: Set<UInt32> = []
  private(set) var recordings: Set<UUID> = []
  var recording: Bool { !recordings.isEmpty }
  private(set) var volumeReaders = 0

  mutating func setAudible(slot: UInt32, _ value: Bool) -> Bool {
    value ? audible.insert(slot).inserted : audible.remove(slot) != nil
  }

  mutating func setRecording(_ id: UUID, _ value: Bool) -> Bool {
    value ? recordings.insert(id).inserted : recordings.remove(id) != nil
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
