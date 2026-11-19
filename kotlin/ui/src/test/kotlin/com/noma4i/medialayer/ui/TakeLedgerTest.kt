package com.noma4i.medialayer.ui

import com.noma4i.medialayer.CoverageVerdict
import com.noma4i.medialayer.audioCoverageRefusal
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** INV-AUDIO-71: the ledger of a take gives the verdict of the core rule from what the writer wrote, the same one the file scan gives for the same sound. */
class TakeLedgerTest {
  private val rate = 44_100
  private val frame = 1024
  private val step = frame * 1_000_000L / rate

  /** A take of ten audio samples whose video runs the whole length, with the audio samples [written] at their slots. */
  private fun take(written: List<Int>, audio: Boolean = true): TakeLedger {
    val ledger = TakeLedger.of(audio, rate, frame)
    ledger.video(0L, 10 * step)
    written.forEach { at -> ledger.audio(at * step, (at + 1) * step, frame) }
    return ledger
  }

  @Test
  fun inv_audio_71_whole_sound_is_covered() {
    val coverage = take((0 until 10).toList()).coverage()
    assertEquals(CoverageVerdict.COVERED, coverage.verdict)
    assertNull(audioCoverageRefusal(coverage.verdict))
  }

  @Test
  fun inv_audio_71_a_ledger_gives_the_verdict_the_scan_gives_for_the_same_damage() {
    assertEquals(CoreCoverage.head().verdict, take((4 until 10).toList()).coverage().verdict)
    assertEquals(CoreCoverage.middle().verdict, take(listOf(0, 1, 2, 3, 7, 8, 9)).coverage().verdict)
    assertEquals(CoreCoverage.tail().verdict, take((0 until 5).toList()).coverage().verdict)
    assertEquals(CoreCoverage.empty().verdict, take(emptyList()).coverage().verdict)
  }

  @Test
  fun inv_audio_71_a_two_frame_take_with_whole_sound_is_covered() {
    val ledger = TakeLedger.of(true, rate, frame)
    ledger.video(0L, 16_667L)
    ledger.video(16_667L, 33_334L)
    ledger.audio(0L, step, frame)
    ledger.audio(step, 2 * step, frame)
    assertEquals(CoverageVerdict.COVERED, ledger.coverage().verdict)
  }

  @Test
  fun inv_audio_71_a_take_with_no_sound_in_its_profile_is_not_refused() {
    val coverage = take(emptyList(), audio = false).coverage()
    assertNull(audioCoverageRefusal(coverage.verdict))
  }

  @Test
  fun inv_audio_71_an_aac_frame_the_encoder_did_not_declare_is_refused_as_broken() {
    val ledger = TakeLedger.of(true, rate, 0)
    ledger.video(0L, 10 * step)
    ledger.audio(0L, step, frame)
    assertEquals(CoverageVerdict.BROKEN, ledger.coverage().verdict)
  }
}
