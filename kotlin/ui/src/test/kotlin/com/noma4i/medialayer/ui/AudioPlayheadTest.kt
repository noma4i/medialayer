package com.noma4i.medialayer.ui

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

  @Test
  fun inv_playhead_10_the_drawn_position_advances_between_reports_and_never_steps_back_on_a_late_one() {
    val control = AudioControl(true) { }

    control.report(report(2000, 0))
    assertEquals(0.25f, control.playhead.progress(nanos(500)), 1e-6f)

    control.report(report(2100, 250))
    assertEquals(0.225f, control.playhead.progress(nanos(250)), 1e-6f)
    assertEquals(0.275f, control.playhead.progress(nanos(750)), 1e-6f)

    control.report(report(2250, 250, rate = 2.0))
    assertEquals(0.325f, control.playhead.progress(nanos(750)), 1e-6f)

    control.report(report(3250, 750, playing = false))
    assertEquals(0.325f, control.playhead.progress(nanos(5000)), 1e-6f)
  }

  @Test
  fun inv_playhead_11_a_seek_moves_the_drawn_position_at_once() {
    val control = AudioControl(true) { }
    control.report(report(2000, 0))

    control.seek(0.8f)

    assertTrue(kotlin.math.abs(control.playhead.progress(System.nanoTime()) - 0.8f) < 0.01f)
  }

  @Test
  fun inv_playhead_12_frames_run_only_while_the_audio_sounds() {
    val control = AudioControl(true) { }
    control.state = MediaState.Ready
    control.report(report(0, 0))
    rule.mainClock.autoAdvance = false
    var frame: Float = -1f
    rule.setContent { frame = rememberPlayheadProgress(control).value }
    rule.mainClock.advanceTimeBy(1000)
    val sounding = frame

    control.pause()
    control.report(report(1000, 1000, playing = false))
    rule.mainClock.advanceTimeBy(100)
    val frozen = frame
    rule.mainClock.advanceTimeBy(2000)

    assertTrue("the position advanced while sounding: $sounding", sounding > 0.05f)
    assertEquals(frozen, frame, 0f)
  }
}
