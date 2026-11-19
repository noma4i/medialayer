package com.noma4i.medialayer.ui

import com.noma4i.medialayer.CameraLens
import com.noma4i.medialayer.CameraPhase
import com.noma4i.medialayer.CameraState
import com.noma4i.medialayer.PlayheadAnchor
import com.noma4i.medialayer.PlayheadReport
import com.noma4i.medialayer.playheadAt
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

private fun cameraState(phase: CameraPhase, recordedMs: ULong, take: ULong?) =
  CameraState(phase, CameraLens.BACK, false, false, 1f, 1f, 1f, recordedMs, false, null, null, take, null)

class PlayheadProgressTest {
  @Test
  fun the_take_report_reads_the_recorded_time_against_the_limit_at_rate_one() {
    val report = takeReport(cameraState(CameraPhase.RECORDING, 300uL, 7uL), 10_000uL, 5_000uL, 7uL)

    assertEquals(300uL, report.positionMs)
    assertEquals(10_000uL, report.durationMs)
    assertEquals(5_000uL, report.atMs)
    assertEquals(1.0, report.rate, 0.0)
    assertTrue(report.playing)
    assertFalse(report.jump)
    assertFalse(report.settling)
  }

  @Test
  fun a_new_take_is_a_jump_and_a_state_out_of_recording_does_not_play() {
    val start = takeReport(cameraState(CameraPhase.RECORDING, 0uL, 8uL), 10_000uL, 0uL, 7uL)
    val ended = takeReport(cameraState(CameraPhase.READY, 4_000uL, 8uL), 10_000uL, 0uL, 8uL)
    val none = takeReport(cameraState(CameraPhase.READY, 0uL, null), 10_000uL, 0uL, 8uL)

    assertTrue(start.jump)
    assertFalse(ended.playing)
    assertFalse(ended.jump)
    assertTrue(none.jump)
  }

  @Test
  fun the_ticks_of_a_take_never_move_the_ring_back_and_reach_the_limit() {
    val limit = 1_000uL
    val reports = ArrayList<PlayheadReport>()
    var previous: ULong? = null
    for (tick in 0..10) {
      val late = if (tick % 2 == 1) 30 else 0
      val position = (tick * 100 - late).coerceAtLeast(0).toULong()
      reports += takeReport(cameraState(CameraPhase.RECORDING, position, 1uL), limit, (tick * 100).toULong(), previous)
      previous = 1uL
    }
    var anchor: PlayheadAnchor? = null
    var last = 0.0
    var now = 0L
    for (report in reports) {
      anchor = com.noma4i.medialayer.playheadNext(anchor, report)
      while (now <= report.atMs.toLong()) {
        val value = playheadAt(anchor, now.toULong())
        assertTrue("ring moved back at $now: $value < $last", value >= last - 1e-9)
        last = value
        now += 16
      }
    }
    assertEquals(1.0, playheadAt(anchor!!, limit), 0.05)
  }

  @Test
  fun the_fold_of_a_video_report_list_follows_the_player_and_a_loop_wrap_is_a_jump() {
    fun video(position: ULong, at: ULong, jump: Boolean = false) = PlayheadReport(position, 10_000uL, at, 1.0, true, jump, false)

    val playing = foldPlayhead(listOf(video(2_000uL, 0uL), video(2_100uL, 100uL)))
    assertEquals(0.21, playheadAt(playing, 100uL), 1e-6)
    assertEquals(0.26, playheadAt(playing, 600uL), 1e-6)

    val wrapped = foldPlayhead(listOf(video(9_900uL, 0uL), video(0uL, 100uL, jump = true)))
    assertEquals(0.0, playheadAt(wrapped, 100uL), 1e-6)

    val paused = foldPlayhead(listOf(video(2_000uL, 0uL), PlayheadReport(2_000uL, 10_000uL, 100uL, 1.0, false, false, false)))
    assertEquals(0.2, playheadAt(paused, 5_000uL), 1e-6)
  }
}
