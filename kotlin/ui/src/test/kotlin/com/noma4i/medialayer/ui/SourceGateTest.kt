package com.noma4i.medialayer.ui

import com.noma4i.medialayer.ui.SourceGate.Config
import com.noma4i.medialayer.ui.SourceGate.Signal
import org.junit.Assert.assertEquals
import org.junit.Test

class SourceGateTest {
  @Test
  fun inv_voice_37_a_source_silenced_before_its_first_pcm_is_told_at_the_first_read() {
    val gate = SourceGate()
    assertEquals(false to Signal.SILENCED, gate.observe(Config.SILENCED, true))
    assertEquals("told once", false to null, gate.observe(Config.SILENCED, true))
    assertEquals("the sound comes back as live", true to Signal.LIVE, gate.observe(Config.ACTIVE, true))
  }

  @Test
  fun inv_voice_37_a_source_with_no_configuration_waits_for_the_platform_callback_that_tells_it_silenced() {
    val gate = SourceGate()
    repeat(1_000) { assertEquals("no configuration is no event", false to null, gate.observe(Config.ABSENT, true)) }
    assertEquals("the recording callback brings the silenced configuration", false to Signal.SILENCED, gate.observe(Config.SILENCED, false))
  }

  @Test
  fun inv_voice_37_a_live_source_tells_silence_and_its_return_once_each() {
    val gate = SourceGate()
    assertEquals(true to Signal.LIVE, gate.observe(Config.ACTIVE, true))
    assertEquals(true to null, gate.observe(Config.ACTIVE, true))
    assertEquals(false to Signal.SILENCED, gate.observe(Config.SILENCED, false))
    assertEquals(false to null, gate.observe(Config.ABSENT, true))
    assertEquals("a callback does not deliver", false to null, gate.observe(Config.ACTIVE, false))
    assertEquals(true to Signal.HEARD, gate.observe(Config.ACTIVE, true))
  }
}
