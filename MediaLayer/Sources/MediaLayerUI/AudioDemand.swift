import Foundation

struct AudioDemand {
  private(set) var audible: Set<UInt32> = []
  private(set) var recordings: Set<UUID> = []
  private(set) var calls: [UUID] = []
  private(set) var managedCalls: Set<UUID> = []
  private(set) var speakers: Set<UUID> = []
  var systemLive = false
  var recording: Bool { !recordings.isEmpty }
  private(set) var volumeReaders = 0

  mutating func setAudible(slot: UInt32, _ value: Bool) -> Bool {
    value ? audible.insert(slot).inserted : audible.remove(slot) != nil
  }

  mutating func setRecording(_ id: UUID, _ value: Bool) -> Bool {
    if value && !calls.isEmpty { return false }
    return value ? recordings.insert(id).inserted : recordings.remove(id) != nil
  }

  mutating func addCall(_ id: UUID, managed: Bool) {
    calls.append(id)
    if managed { managedCalls.insert(id) }
  }

  mutating func removeCall(_ id: UUID) {
    calls.removeAll { $0 == id }
    managedCalls.remove(id)
    speakers.remove(id)
    if calls.isEmpty { systemLive = false }
  }

  mutating func setSpeaker(_ id: UUID, _ on: Bool) {
    guard calls.contains(id) else { return }
    if on { speakers.insert(id) } else { speakers.remove(id) }
  }

  mutating func readVolume(_ delta: Int) {
    volumeReaders = max(0, volumeReaders + delta)
  }

  func target(for focus: MediaAudioFocus) -> SessionTarget {
    if let call = calls.last { return .call(.call(speaker: speakers.contains(call)), managedBySystem: !managedCalls.isEmpty) }
    if recording { return .active(.recording) }

    if !audible.isEmpty { return .active(.playback(focus)) }

    return volumeReaders > 0 ? .active(.playback(.mix)) : .idle
  }
}
