package com.noma4i.medialayer.ui

import androidx.media3.common.Player
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.assertNull
import org.junit.Test

class AudioDemandTest {
  private val demand = AudioDemand()

  @Test
  fun noAudibleSlotMeansNoFocusUnderEveryPolicy() {
    MediaAudioFocus.entries.forEach { assertNull(demand.target(it)) }
  }

  @Test
  fun anAudibleSlotAsksForTheDuckFocusOnlyUnderDuck() {
    demand.setAudible(1u, true)

    assertEquals(FocusKind.Duck, demand.target(MediaAudioFocus.Duck))
    assertNull(demand.target(MediaAudioFocus.Mix))
    assertNull(demand.target(MediaAudioFocus.Exclusive))
  }

  @Test
  fun theFocusStaysUntilTheLastAudibleSlotLeaves() {
    demand.setAudible(1u, true)
    demand.setAudible(2u, true)
    demand.setAudible(1u, false)

    assertEquals(FocusKind.Duck, demand.target(MediaAudioFocus.Duck))

    demand.setAudible(2u, false)

    assertNull(demand.target(MediaAudioFocus.Duck))
  }

  @Test
  fun recordingWinsOverPlaybackUnderEveryPolicyAndPlaybackReturnsAfterIt() {
    demand.setAudible(1u, true)
    demand.setRecording(true)

    MediaAudioFocus.entries.forEach { assertEquals(FocusKind.Recording, demand.target(it)) }

    demand.setRecording(false)

    assertEquals(FocusKind.Duck, demand.target(MediaAudioFocus.Duck))
  }

  @Test
  fun aSlotIsAudibleOnlyWhenItReallyPlaysWithSoundAndHoldsAnItem() {
    assertTrue(isAudible(1f, true, Player.STATE_READY, true))
    assertTrue(isAudible(1f, true, Player.STATE_BUFFERING, true))
    assertFalse("muted", isAudible(0f, true, Player.STATE_READY, true))
    assertFalse("paused", isAudible(1f, false, Player.STATE_READY, true))
    assertFalse("ended", isAudible(1f, true, Player.STATE_ENDED, true))
    assertFalse("unloaded", isAudible(1f, true, Player.STATE_IDLE, false))
  }
}
