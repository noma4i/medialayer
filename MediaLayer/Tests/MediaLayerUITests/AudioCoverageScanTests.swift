#if os(iOS)
import Foundation
import MediaLayer
import Testing
@testable import MediaLayerUI

@Suite(.timeLimit(.minutes(1)))
struct AudioCoverageScanTests {
  @Test
  func inv_capture_50_a_real_aac_file_with_video_is_covered_by_its_decoded_pcm() async throws {
    let url = try await CoverageClips.write(audio: CoverageClips.whole)
    defer { try? FileManager.default.removeItem(at: url) }
    let coverage = try await scanAudioCoverage(at: url, required: true)
    #expect(coverage.verdict == .covered)
    #expect(coverage.samples >= 9_000 && coverage.samples <= 10_500)
    #expect(audioCoverageRefusal(verdict: coverage.verdict) == nil)
  }

  // The AAC encoder of the writer closes a hole between PCM buffers (no silence is made), so a lost middle is a short stream: 140 ms of 200 ms, one run, the missing part at the tail.
  @Test(arguments: [(CoverageClips.hole, 6_000...7_500, 30_000...90_000), (CoverageClips.cut, 3_000...4_800, 70_000...130_000)])
  func inv_capture_50_a_track_whose_pcm_does_not_cover_the_middle_or_the_tail_is_a_silent_microphone(_ indexes: [Int], _ samples: ClosedRange<UInt64>, _ lost: ClosedRange<Int64>) async throws {
    let url = try await CoverageClips.write(audio: indexes)
    defer { try? FileManager.default.removeItem(at: url) }
    let coverage = try await scanAudioCoverage(at: url, required: true)
    #expect(coverage.verdict == .tailCut)
    #expect(audioCoverageRefusal(verdict: coverage.verdict) == .audioTrackBroken)
    #expect(samples.contains(coverage.samples), "\(coverage)")
    let span = Double((coverage.lastUs ?? 0) - (coverage.firstUs ?? 0))
    #expect(abs(span - Double(coverage.samples) / 48_000 * 1_000_000) <= 25_000, "no hole inside the decoded stream: \(coverage)")
    #expect(coverage.gapAtUs == coverage.lastUs && lost.contains(coverage.gapUs ?? 0), "\(coverage)")
  }

  @Test
  func inv_capture_50_a_profile_without_audio_decodes_nothing_and_a_file_that_cannot_be_read_is_broken() async throws {
    let missing = FileManager.default.temporaryDirectory.appendingPathComponent("none-\(UUID().uuidString).mp4")
    #expect(try await scanAudioCoverage(at: missing, required: false).verdict == .skipped)
    let junk = FileManager.default.temporaryDirectory.appendingPathComponent("junk-\(UUID().uuidString).mp4")
    try Data([1, 2, 3]).write(to: junk)
    defer { try? FileManager.default.removeItem(at: junk) }
    let broken = try await scanAudioCoverage(at: junk, required: true)
    #expect(audioCoverageRefusal(verdict: broken.verdict) == .audioTrackBroken)
  }
}
#endif
