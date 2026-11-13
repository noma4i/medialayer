import Foundation
import MediaLayer
import Testing
@testable import MediaLayerUI

private let origin: UInt64 = 100_000_000

private func report(_ positionMs: UInt64, at offsetMs: UInt64, rate: Double = 1, playing: Bool = true, settling: Bool = false) -> PlayheadReport {
  PlayheadReport(positionMs: positionMs, durationMs: 10_000, atMs: origin + offsetMs, rate: rate, playing: playing, jump: false, settling: settling)
}

private func at(_ offsetMs: UInt64) -> Date { Date(timeIntervalSince1970: Double(origin + offsetMs) / 1000) }

@MainActor private func note() -> AudioPlayback {
  AudioPlayback(item: MediaItem(kind: .audio, url: "note", headers: [:], cover: nil, blurHash: nil, strategy: .window), playing: true, places: RecordingPlaces())
}

struct AudioPlayheadTests {
  @MainActor
  @Test
  func inv_playhead_10_theDrawnPositionAdvancesBetweenReportsAndNeverStepsBackOnALateOne() {
    let audio = note()

    audio.refresh(report: report(2000, at: 0))
    #expect(abs(audio.playhead.progress(at: at(500)) - 0.25) < 1e-6)

    audio.refresh(report: report(2100, at: 250))
    #expect(abs(audio.playhead.progress(at: at(250)) - 0.225) < 1e-6)
    #expect(abs(audio.playhead.progress(at: at(750)) - 0.275) < 1e-6)

    audio.refresh(report: report(2250, at: 250, rate: 2))
    #expect(abs(audio.playhead.progress(at: at(750)) - 0.325) < 1e-6)

    audio.refresh(report: report(3250, at: 750, playing: false))
    #expect(abs(audio.playhead.progress(at: at(5000)) - 0.325) < 1e-6)
  }

  @MainActor
  @Test
  func inv_playhead_11_aSeekMovesTheDrawnPositionAtOnceAndAReportOfTheOldPositionWhileItIsInFlightDoesNotPullItBack() {
    let audio = note()
    audio.refresh(report: report(2000, at: 0))

    audio.seek(to: 0.8)
    let sought = audio.playhead.progress()
    audio.refresh(report: PlayheadReport(positionMs: 2200, durationMs: 10_000, atMs: UInt64(Date().timeIntervalSince1970 * 1000), rate: 1, playing: true, jump: false, settling: true))

    #expect(abs(sought - 0.8) < 0.01)
    #expect(abs(audio.playhead.progress() - 0.8) < 0.01)
  }
}
