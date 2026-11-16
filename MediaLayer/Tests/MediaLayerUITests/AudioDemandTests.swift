import Foundation
import Testing
@testable import MediaLayerUI

struct AudioDemandTests {
  @Test
  func targetFollowsFocusAudibleSlotsAndRecording() {
    var demand = AudioDemand()
    #expect(demand.target(for: .duck) == .idle)

    _ = demand.setAudible(slot: 1, true)
    _ = demand.setAudible(slot: 2, true)
    #expect(demand.target(for: .mix) == .active(.playback(.mix)))
    #expect(demand.target(for: .exclusive) == .active(.playback(.exclusive)))

    _ = demand.setAudible(slot: 1, false)
    #expect(demand.target(for: .mix) != .idle)
    _ = demand.setAudible(slot: 2, false)
    #expect(demand.target(for: .mix) == .idle)

    _ = demand.setRecording(UUID(), true)
    #expect(demand.target(for: .duck) == .active(.recording))
  }

  @Test
  func volumeReadersHoldANonDuckingSessionOnlyWhileNothingIsHeard() {
    var demand = AudioDemand()
    demand.readVolume(1)
    #expect(demand.target(for: .duck) == .active(.playback(.mix)))

    _ = demand.setAudible(slot: 1, true)
    #expect(demand.target(for: .duck) == .active(.playback(.duck)))
    _ = demand.setAudible(slot: 1, false)
    demand.readVolume(-1)
    demand.readVolume(-1)
    #expect(demand.target(for: .duck) == .idle)
  }

  @Test
  func thePlaybackOptionSetsDifferOnlyByDucking() {
    #expect(SessionPlan.playback(.exclusive).options.isEmpty)
    #expect(SessionPlan.playback(.mix).options == [.mixWithOthers])
    #expect(SessionPlan.playback(.duck).options == [.mixWithOthers, .duckOthers])
    #expect(SessionPlan.recording == SessionPlan(category: .playAndRecord, mode: .default, options: [.defaultToSpeaker]))
  }

  @Test
  func changesReportOnlyRealTransitions() {
    var demand = AudioDemand()
    let id = UUID()
    let changes = [
      demand.setAudible(slot: 1, true), demand.setAudible(slot: 1, true),
      demand.setAudible(slot: 1, false), demand.setAudible(slot: 1, false),
      demand.setRecording(id, true), demand.setRecording(id, true)
    ]
    #expect(changes == [true, false, true, false, true, false])
  }
}
