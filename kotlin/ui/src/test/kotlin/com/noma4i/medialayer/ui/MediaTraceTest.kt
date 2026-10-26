package com.noma4i.medialayer.ui

import android.util.Log
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.shadows.ShadowLog

@RunWith(MediaLayerTestRunner::class)
class MediaTraceTest {
  @After
  fun off() {
    MediaTrace.reset()
    ShadowLog.reset()
  }

  private fun lines() = ShadowLog.getLogsForTag(MediaTrace.TAG).map { it.msg }

  @Test
  fun inv_trace_02_without_the_platform_flag_the_trace_is_off_and_writes_nothing() {
    MediaTrace.install()
    MediaTrace.emit("player") { error("the text is built only when the trace is on") }

    assertFalse(MediaTrace.isOn)
    assertEquals(emptyList<String>(), lines())
  }

  @Test
  fun inv_trace_03_the_logcat_flag_of_the_tag_turns_the_trace_on_and_lines_reach_logcat_under_one_tag() {
    ShadowLog.setLoggable(MediaTrace.TAG, Log.VERBOSE)

    MediaTrace.install()
    MediaTrace.emit("player") { "slot=1 ended generation=7" }
    MediaTrace.Logcat.line("ML 5 stage ended slot=1 generation=7")

    assertTrue(MediaTrace.isOn)
    val seen = lines()
    assertEquals(2, seen.size)
    assertTrue(seen[0], Regex("""ML \d+ player slot=1 ended generation=7""").matches(seen[0]))
    assertEquals("ML 5 stage ended slot=1 generation=7", seen[1])
  }
}
