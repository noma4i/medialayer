package com.noma4i.medialayer.ui

import com.noma4i.medialayer.ui.SourceGate.Config
import com.noma4i.medialayer.ui.SourceGate.Signal
import org.junit.Assert.assertEquals
import org.junit.Test

class SourceGateTest {
  @Test
  fun inv_voice_37_a_source_silenced_before_its_first_pcm_is_told_at_the_first_read() {
    val gate = SourceGate(44_100)
    assertEquals(false to Signal.SILENCED, gate.observe(Config.SILENCED, true, 1_024))
    assertEquals("told once", false to null, gate.observe(Config.SILENCED, true, 1_024))
    assertEquals("the sound comes back as live", true to Signal.LIVE, gate.observe(Config.ACTIVE, true, 1_024))
  }

  @Test
  fun inv_voice_37_a_source_with_no_configuration_is_told_silenced_after_a_second_of_reads() {
    val gate = SourceGate(44_100)
    repeat(43) { assertEquals(false to null, gate.observe(Config.ABSENT, true, 1_024)) }
    assertEquals("a callback is no read", false to null, gate.observe(Config.ABSENT, false, 0))
    assertEquals(false to Signal.SILENCED, gate.observe(Config.ABSENT, true, 1_024))
  }

  @Test
  fun inv_voice_37_a_live_source_tells_silence_and_its_return_once_each() {
    val gate = SourceGate(44_100)
    assertEquals(true to Signal.LIVE, gate.observe(Config.ACTIVE, true, 1_024))
    assertEquals(true to null, gate.observe(Config.ACTIVE, true, 1_024))
    assertEquals(false to Signal.SILENCED, gate.observe(Config.SILENCED, false, 0))
    assertEquals(false to null, gate.observe(Config.ABSENT, true, 1_024))
    assertEquals("a callback does not deliver", false to null, gate.observe(Config.ACTIVE, false, 0))
    assertEquals(true to Signal.HEARD, gate.observe(Config.ACTIVE, true, 1_024))
  }
}
