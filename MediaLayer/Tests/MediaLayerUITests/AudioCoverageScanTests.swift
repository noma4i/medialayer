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

  // The AAC encoder of the writer closes a hole between PCM buffers (the stream gets shorter by the hole, no silence is made), so a lost middle shows as a short tail.
  @Test(arguments: [(CoverageClips.hole, CoverageVerdict.tailCut), (CoverageClips.cut, CoverageVerdict.tailCut)])
  func inv_capture_50_a_track_whose_pcm_does_not_cover_the_middle_or_the_tail_is_a_silent_microphone(_ indexes: [Int], _ verdict: CoverageVerdict) async throws {
    let url = try await CoverageClips.write(audio: indexes)
    defer { try? FileManager.default.removeItem(at: url) }
    let coverage = try await scanAudioCoverage(at: url, required: true)
    #expect(coverage.verdict == verdict)
    #expect(audioCoverageRefusal(verdict: coverage.verdict) == .microphoneSilent)
  }

  @Test
  func inv_capture_50_the_encoder_closes_a_pcm_hole_so_the_stream_is_contiguous_and_shorter_by_the_hole() async throws {
    let url = try await CoverageClips.write(audio: CoverageClips.hole)
    defer { try? FileManager.default.removeItem(at: url) }
    let coverage = try await scanAudioCoverage(at: url, required: true)
    let whole = Double(coverage.samples) / 48_000 * 1_000_000
    let span = Double((coverage.lastUs ?? 0) - (coverage.firstUs ?? 0))
    #expect((6_000...7_500).contains(coverage.samples), "140 ms of PCM reached the encoder, not 200 ms: \(coverage)")
    #expect(abs(span - whole) <= 25_000, "the decoded stream has no hole inside: \(coverage)")
    #expect(coverage.gapAtUs == coverage.lastUs && (30_000...90_000).contains(coverage.gapUs ?? 0), "the lost 60 ms stands at the tail: \(coverage)")
  }

  @Test
  func inv_capture_50_a_profile_without_audio_decodes_nothing_and_a_file_that_cannot_be_read_is_broken() async throws {
    let missing = FileManager.default.temporaryDirectory.appendingPathComponent("none-\(UUID().uuidString).mp4")
    #expect(try await scanAudioCoverage(at: missing, required: false).verdict == .skipped)
    let junk = FileManager.default.temporaryDirectory.appendingPathComponent("junk-\(UUID().uuidString).mp4")
    try Data([1, 2, 3]).write(to: junk)
    defer { try? FileManager.default.removeItem(at: junk) }
    let broken = try await scanAudioCoverage(at: junk, required: true)
    #expect(audioCoverageRefusal(verdict: broken.verdict) == .microphoneSilent)
  }
}
#endif
