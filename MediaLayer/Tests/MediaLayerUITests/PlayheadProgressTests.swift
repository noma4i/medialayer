import MediaLayer
import Testing
@testable import MediaLayerUI

private func state(_ phase: CameraPhase, recordedMs: UInt64, take: UInt64?) -> CameraState {
  CameraState(phase: phase, lens: .back, flash: false, hasFlash: false, zoom: 1, zoomMin: 1, zoomMax: 1, recordedMs: recordedMs, shooting: false, fault: nil, refusedMs: nil, take: take, outcome: nil)
}

struct PlayheadProgressTests {
  @Test
  func inv_playhead_15_aRecordingStateBecomesAPlayingReportAgainstTheProfileLimit() {
    let report = takeReport(state: state(.recording, recordedMs: 300, take: 7), limitMs: 60_000, nowMs: 1_000, previousTake: 7)

    #expect(report.positionMs == 300)
    #expect(report.durationMs == 60_000)
    #expect(report.atMs == 1_000)
    #expect(report.rate == 1)
    #expect(report.playing)
    #expect(!report.jump)
    #expect(!report.settling)
  }

  @Test
  func inv_playhead_15_aNewTakeJumpsAndAFinishedTakeStopsTheRing() {
    let start = takeReport(state: state(.recording, recordedMs: 0, take: 8), limitMs: 60_000, nowMs: 1_000, previousTake: 7)
    let end = takeReport(state: state(.stopping, recordedMs: 5_000, take: 8), limitMs: 60_000, nowMs: 6_000, previousTake: 8)

    #expect(start.jump)
    #expect(!end.playing)
    #expect(!end.jump)
    #expect(playheadNext(shown: playheadNext(shown: nil, report: start), report: end).perMs == 0)
  }

  @MainActor
  @Test
  func inv_playhead_15_aPositionBehindThePreviousOneWhilePlayingIsALoopWrap() {
    #expect(PlaybackProgress.wrapped(previousMs: 9_900, positionMs: 40, playing: true, seeking: false))
    #expect(!PlaybackProgress.wrapped(previousMs: 9_900, positionMs: 40, playing: true, seeking: true))
    #expect(!PlaybackProgress.wrapped(previousMs: 9_900, positionMs: 40, playing: false, seeking: false))
    #expect(!PlaybackProgress.wrapped(previousMs: nil, positionMs: 40, playing: true, seeking: false))
    #expect(!PlaybackProgress.wrapped(previousMs: 1_000, positionMs: 1_100, playing: true, seeking: false))
  }
}
