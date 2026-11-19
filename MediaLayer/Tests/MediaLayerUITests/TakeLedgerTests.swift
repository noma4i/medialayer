#if os(iOS)
import CoreGraphics
import Foundation
import ImageIO
import MediaLayer
import Testing
@testable import MediaLayerUI

struct TakeLedgerTests {
  private static let rate: UInt32 = 48_000
  private static let frame: UInt32 = 1024

  /// The PCM buffers of 1024 samples at 48 kHz the writer hands over, by index, each at its own time.
  private static func buffer(_ index: Int) -> AudioInterval {
    let start = Int64((Double(index) * 1024 / 48_000 * 1_000_000).rounded())
    let end = Int64((Double(index + 1) * 1024 / 48_000 * 1_000_000).rounded())
    return AudioInterval(startUs: start, endUs: end, samples: 1024)
  }

  private static func ledger(audio indexes: [Int], videoEndUs: Int64 = 2_000_000) -> CoverageVerdict {
    var ledger = TakeLedger(audio: (rate, frame))
    for index in indexes { ledger.audio(buffer(index)) }
    ledger.video(startUs: 0, endUs: videoEndUs)
    return ledger.coverage().verdict
  }

  @Test
  func inv_capture_50_a_ledger_of_whole_sound_is_covered() {
    #expect(Self.ledger(audio: Array(0..<94)) == .covered)
  }

  @Test
  func inv_capture_50_a_ledger_without_the_head_the_middle_or_the_tail_gives_the_refusal_of_the_file_scan() {
    let head = Self.ledger(audio: Array(10..<94))
    let middle = Self.ledger(audio: Array(0..<40) + Array(46..<94))
    let tail = Self.ledger(audio: Array(0..<90))
    #expect([head, middle, tail] == [.headGap, .midGap, .tailCut])
    for verdict in [head, middle, tail] { #expect(audioCoverageRefusal(verdict: verdict) == .audioTrackBroken) }
  }

  @Test
  func inv_capture_50_a_two_frame_take_with_whole_sound_is_covered_and_a_take_without_sound_in_its_profile_is_not_refused() {
    var take = TakeLedger(audio: (Self.rate, Self.frame))
    for index in 0..<4 { take.audio(Self.buffer(index)) }
    take.video(startUs: 0, endUs: 33_333)
    take.video(startUs: 33_333, endUs: 66_667)
    #expect(take.coverage().verdict == .covered)

    var quiet = TakeLedger(audio: nil)
    quiet.video(startUs: 0, endUs: 2_000_000)
    #expect(audioCoverageRefusal(verdict: quiet.coverage().verdict) == nil)
  }

  @Test
  func inv_capture_50_a_ledger_that_heard_no_sound_or_no_picture_is_broken() {
    #expect(Self.ledger(audio: []) == .empty)
    var noPicture = TakeLedger(audio: (Self.rate, Self.frame))
    noPicture.audio(Self.buffer(0))
    #expect(noPicture.coverage().verdict == .broken)
  }

  @Test
  func inv_capture_50_the_writer_judges_the_sound_it_was_handed_whole_hole_and_cut() async throws {
    let whole = try await CoverageClips.writeJudged(audio: CoverageClips.whole)
    let hole = try await CoverageClips.writeJudged(audio: CoverageClips.hole)
    let cut = try await CoverageClips.writeJudged(audio: CoverageClips.cut)
    defer { [whole.url, hole.url, cut.url].forEach { try? FileManager.default.removeItem(at: $0) } }
    #expect([whole.coverage.verdict, hole.coverage.verdict, cut.coverage.verdict] == [.covered, .midGap, .tailCut])
    // The same verdict the file scan gives for the same damage.
    let scanned = try await scanAudioCoverage(at: hole.url, required: true)
    #expect(audioCoverageRefusal(verdict: scanned.verdict) == audioCoverageRefusal(verdict: hole.coverage.verdict))
  }

  @Test
  func inv_capture_82_upright_frames_take_an_identity_transform_and_raw_frames_keep_the_turn_of_the_take() {
    let raw = FrameTurn()
    let upright = FrameTurn(upright: true, mirrored: false)
    #expect(takeWriterTransform(turn: upright, mirror: true) == .identity)
    #expect(takeWriterTransform(turn: raw, mirror: false) == takeTransform(mirror: false))
    #expect(takeWriterTransform(turn: raw, mirror: true) == takeTransform(mirror: true))
  }

  @Test
  func inv_capture_82_the_tap_orientation_follows_the_turn_of_the_frames_and_the_mirror_of_the_front_lens() {
    #expect(tapOrientation(lens: .back, turn: FrameTurn()) == .right)
    #expect(tapOrientation(lens: .front, turn: FrameTurn()) == .leftMirrored)
    #expect(tapOrientation(lens: .back, turn: FrameTurn(upright: true, mirrored: false)) == .up)
    #expect(tapOrientation(lens: .front, turn: FrameTurn(upright: true, mirrored: true)) == .up)
    #expect(tapOrientation(lens: .front, turn: FrameTurn(upright: true, mirrored: false)) == .upMirrored)
  }

  @Test
  func inv_capture_82_the_connection_is_turned_only_when_it_can_do_the_whole_turn() {
    #expect(turnsFramesUpright(angleSupported: false, mirroringSupported: true, mirrorWanted: false) == false)
    #expect(turnsFramesUpright(angleSupported: true, mirroringSupported: false, mirrorWanted: true) == false)
    #expect(turnsFramesUpright(angleSupported: true, mirroringSupported: false, mirrorWanted: false))
    #expect(turnsFramesUpright(angleSupported: true, mirroringSupported: true, mirrorWanted: true))
  }
}
#endif
