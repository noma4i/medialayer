import MediaLayer
import Testing
@testable import MediaLayerUI

private func state(_ phase: CameraPhase, recordedMs: UInt64, take: UInt64?) -> CameraState {
  CameraState(phase: phase, lens: .back, flash: false, hasFlash: false, zoom: 1, zoomMin: 1, zoomMax: 1, recordedMs: recordedMs, shooting: false, fault: nil, refusedMs: nil, take: take, outcome: nil)
}

private func voice(_ phase: VoicePhase, recordedMs: UInt64, take: UInt64) -> VoiceState {
  VoiceState(take: take, phase: phase, outcome: nil, recordedMs: recordedMs)
}

struct PlayheadProgressTests {
  @Test
  func inv_playhead_15_aRecordingStateBecomesAPlayingReportAgainstTheProfileLimit() {
    let report = takeReport(recordedMs: 300, recording: .recording == .recording, take: 7, limitMs: 60_000, nowMs: 1_000, previousTake: 7)

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
    let start = takeReport(recordedMs: 0, recording: .recording == .recording, take: 8, limitMs: 60_000, nowMs: 1_000, previousTake: 7)
    let end = takeReport(recordedMs: 5_000, recording: .stopping == .recording, take: 8, limitMs: 60_000, nowMs: 6_000, previousTake: 8)

    #expect(start.jump)
    #expect(!end.playing)
    #expect(!end.jump)
    #expect(playheadNext(shown: playheadNext(shown: nil, report: start), report: end).perMs == 0)
  }

  @Test
  func inv_playhead_16_aVoiceStateGoesThroughTheSameReportBuilder() {
    let recording = takeReport(recordedMs: 300, recording: voice(.recording, recordedMs: 300, take: 3).phase == .recording, take: 3, limitMs: 120_000, nowMs: 1_000, previousTake: 3)
    let fresh = takeReport(recordedMs: 0, recording: true, take: 4, limitMs: 120_000, nowMs: 2_000, previousTake: 3)
    let idle = takeReport(recordedMs: 0, recording: voice(.idle, recordedMs: 0, take: 4).phase == .recording, take: 4, limitMs: 120_000, nowMs: 3_000, previousTake: 4)

    #expect(recording.playing && !recording.jump && recording.positionMs == 300 && recording.durationMs == 120_000)
    #expect(fresh.jump)
    #expect(!idle.playing && !idle.jump)
    #expect(playheadNext(shown: playheadNext(shown: nil, report: fresh), report: idle).perMs == 0)
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
