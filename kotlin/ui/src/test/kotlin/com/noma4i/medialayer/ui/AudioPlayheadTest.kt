package com.noma4i.medialayer.ui

import org.robolectric.Shadows.shadowOf
import android.os.Looper
import com.noma4i.medialayer.PlaybackState
import androidx.activity.ComponentActivity
import androidx.compose.runtime.getValue
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import com.noma4i.medialayer.MediaState
import com.noma4i.medialayer.PlayheadReport
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

private fun report(positionMs: Long, atMs: Long, rate: Double = 1.0, playing: Boolean = true, settling: Boolean = false) =
  PlayheadReport(positionMs.toULong(), 10_000uL, atMs.toULong(), rate, playing, false, settling)

private fun nanos(ms: Long) = ms * 1_000_000

@RunWith(MediaLayerTestRunner::class)
class AudioPlayheadTest {
  @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

  private fun AudioControl.reported(value: PlayheadReport) {
    val before = playhead
    report(value)
    rule.waitUntil(5_000) { shadowOf(Looper.getMainLooper()).idle(); playhead !== before }
  }

  @Test
  fun inv_playhead_10_the_drawn_position_advances_between_reports_and_never_steps_back_on_a_late_one() {
    val control = AudioControl(true) { }
    control.playback(PlaybackState(true, true, null, null, 0uL))

    control.reported(report(2000, 0))
    assertEquals(0.25f, control.playhead.progress(nanos(500)), 1e-6f)

    control.reported(report(2100, 250))
    assertEquals(0.225f, control.playhead.progress(nanos(250)), 1e-6f)
    assertEquals(0.275f, control.playhead.progress(nanos(750)), 1e-6f)

    control.reported(report(2250, 250, rate = 2.0))
    assertEquals(0.325f, control.playhead.progress(nanos(750)), 1e-6f)

    control.reported(report(3250, 750, playing = false))
    assertEquals(0.325f, control.playhead.progress(nanos(5000)), 1e-6f)
  }

  @Test
  fun inv_playhead_11_a_seek_moves_the_drawn_position_at_once() {
    val control = AudioControl(true) { }
    control.playback(PlaybackState(true, true, null, null, 0uL))
    control.reported(report(2000, 0))

    control.seek(0.8f)

    assertTrue(kotlin.math.abs(control.playhead.progress(System.nanoTime()) - 0.8f) < 0.01f)
  }

  @Test
  fun inv_playhead_13_a_report_with_an_unknown_duration_keeps_the_progress_and_the_shown_position() {
    val control = AudioControl(true) { }
    control.playback(PlaybackState(true, true, null, null, 0uL))
    control.reported(report(2000, 0))

    control.reported(PlayheadReport(0uL, 0uL, 250uL, 1.0, false, false, false))

    assertEquals(0.2f, control.progress, 1e-6f)
    assertEquals(0.225f, control.playhead.progress(nanos(250)), 1e-6f)
    assertEquals(0.225f, control.playhead.progress(nanos(5000)), 1e-6f)
  }

  @Test
  fun inv_playhead_12_frames_run_only_while_the_audio_sounds() {
    val control = AudioControl(true) { }
    control.playback(PlaybackState(true, true, null, null, 0uL))
    control.state = MediaState.Ready
    control.reported(report(0, 0))
    rule.mainClock.autoAdvance = false
    var frame: Float = -1f
    rule.setContent { frame = rememberPlayheadProgress(control).value }
    rule.mainClock.advanceTimeBy(1000)
    val sounding = frame

    control.pause()
    control.playback(PlaybackState(false, false, null, null, 0uL))
    control.reported(report(1000, 1000, playing = false))
    rule.mainClock.advanceTimeBy(100)
    val frozen = frame
    rule.mainClock.advanceTimeBy(2000)

    assertTrue("the position advanced while sounding: $sounding", sounding > 0.05f)
    assertEquals(frozen, frame, 0f)
  }

  @Test
  fun inv_playhead_14_reports_that_arrive_before_main_draws_chain_like_reports_drawn_one_by_one() {
    val reports = listOf(report(2000, 0), report(2100, 250), report(2250, 250, rate = 2.0))
    val oneByOne = AudioControl(true) { }
    oneByOne.playback(PlaybackState(true, true, null, null, 0uL))
    reports.forEach { oneByOne.reported(it) }
    val expected = oneByOne.playhead.progress(nanos(750))
    val together = AudioControl(true) { }
    together.playback(PlaybackState(true, true, null, null, 0uL))

    reports.forEach { together.report(it) }

    rule.waitUntil(5_000) { shadowOf(Looper.getMainLooper()).idle(); kotlin.math.abs(together.playhead.progress(nanos(750)) - expected) < 1e-6f }
    assertEquals(0.325f, together.playhead.progress(nanos(750)), 1e-6f)
  }
}
