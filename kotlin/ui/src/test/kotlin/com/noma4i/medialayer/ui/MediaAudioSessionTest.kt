package com.noma4i.medialayer.ui

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class MediaAudioSessionTest {
  private val port = FakeFocusPort()
  private val scope = TestScope()

  private fun session(focus: MediaAudioFocus) = MediaAudioSession(focus, port, 300, scope)

  private fun advance(ms: Long) {
    scope.advanceTimeBy(ms)
    scope.runCurrent()
  }

  @Test
  fun mixNeverRequestsFocusAndNeverAbandonsIt() {
    val session = session(MediaAudioFocus.Mix)

    session.setAudible(1u, true)
    session.setAudible(1u, false)
    advance(1000)

    assertEquals(emptyList<String>(), port.calls.toList())
  }

  @Test
  fun exclusiveLeavesTheFocusToThePlayer() {
    val session = session(MediaAudioFocus.Exclusive)

    session.setAudible(1u, true)
    advance(1000)

    assertEquals(emptyList<String>(), port.calls.toList())
  }

  @Test
  fun onlyTheFirstAudibleSlotRequestsTheDuckFocus() {
    val session = session(MediaAudioFocus.Duck)

    session.setAudible(1u, true)
    session.setAudible(2u, true)

    assertEquals(listOf("request:Duck"), port.calls.toList())
  }

  @Test
  fun theLastSlotAbandonsTheFocusAfter300Ms() {
    val session = session(MediaAudioFocus.Duck)
    session.setAudible(1u, true)
    session.setAudible(2u, true)
    session.setAudible(1u, false)
    advance(1000)

    assertEquals(listOf("request:Duck"), port.calls.toList())

    session.setAudible(2u, false)
    advance(299)

    assertEquals(listOf("request:Duck"), port.calls.toList())

    advance(1)

    assertEquals(listOf("request:Duck", "abandon"), port.calls.toList())
  }

  @Test
  fun aPagerThatPausesOnePageAndPlaysTheNextNeverReleasesTheFocus() {
    val session = session(MediaAudioFocus.Duck)
    session.setAudible(1u, true)
    session.setAudible(1u, false)
    advance(150)
    session.setAudible(2u, true)
    advance(1000)

    assertEquals(listOf("request:Duck"), port.calls.toList())
  }

  @Test
  fun recordingTakesThePriorityAndPlaybackFocusReturnsAfterIt() {
    val session = session(MediaAudioFocus.Duck)
    session.setAudible(1u, true)

    assertTrue(session.activateRecording())
    assertTrue(session.isRecording)
    session.deactivateRecording()
    advance(1000)

    assertFalse(session.isRecording)

    assertEquals(listOf("request:Duck", "request:Recording", "request:Duck"), port.calls.toList())
  }

  @Test
  fun recordingWithoutPlaybackReleasesTheFocusAfterItEnds() {
    val session = session(MediaAudioFocus.Mix)

    assertTrue(session.activateRecording())
    session.deactivateRecording()
    advance(300)

    assertFalse(session.isRecording)
    assertEquals(listOf("request:Recording", "abandon"), port.calls.toList())
  }

  @Test
  fun aRefusedRecordingFocusLeavesNoRecordingDemand() {
    port.grant = false
    val session = session(MediaAudioFocus.Mix)

    assertFalse(session.activateRecording())
    assertFalse(session.isRecording)
  }
}
