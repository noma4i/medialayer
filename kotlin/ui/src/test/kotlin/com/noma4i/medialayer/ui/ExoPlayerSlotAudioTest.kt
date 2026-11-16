@file:Suppress("DEPRECATION")

package com.noma4i.medialayer.ui

import android.content.Context
import android.media.AudioManager
import kotlinx.coroutines.ExperimentalCoroutinesApi
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(MediaLayerTestRunner::class)
internal class ExoPlayerSlotAudioTest : ExoPlayerSlotRig() {
  @Test
  fun muteAndUnmuteChangeOnlyTheVolumeAndNeverTouchAudioFocus() {
    val slot = factory(startMuted = false, focus = MediaAudioFocus.Exclusive).createSlot(0u)
    val audio = shadowOf(context.getSystemService(Context.AUDIO_SERVICE) as AudioManager)
    slot.load("k", route); slot.play(); idle()
    TestSignal.await("link focus", 5_000) { focusPort.calls.contains("request GAIN MEDIA") }
    assertNull(audio.lastAudioFocusRequest)
    assertEquals(listOf("request GAIN MEDIA"), focusPort.calls.toList())

    repeat(10) { slot.mute(); idle(); assertEquals(0f, slot.player!!.volume, 0f); slot.unmute(); idle(); assertEquals(1f, slot.player!!.volume, 0f) }
    TestSignal.await("focus after volume changes", 5_000) { focusPort.calls.size == 21 }
    focusPort.calls.clear()
    slot.unload(); idle()
    TestSignal.await("focus abandoned after unload", 5_000) { focusPort.calls.toList() == listOf("abandon") }
    assertEquals(listOf("abandon"), focusPort.calls.toList())
    assertNull(audio.lastAudioFocusRequest)
  }

  @Test
  fun mixPlaysWithSoundAndNeverRequestsFocusFromThePlayerOrTheLibrary() {
    val slot = factory(startMuted = false, focus = MediaAudioFocus.Mix).createSlot(0u)
    val audio = shadowOf(context.getSystemService(Context.AUDIO_SERVICE) as AudioManager)

    slot.load("k", route); slot.play(); settleAudioFocus(); advanceFocus(1000)

    assertTrue(slot.player!!.playWhenReady)
    assertNull(audio.lastAudioFocusRequest)
    assertEquals(emptyList<String>(), focusPort.calls.toList())
  }

  @Test
  fun duckAsksTheLibraryForTheFocusOnceAndThePlayerNever() {
    val slot = factory(startMuted = false, focus = MediaAudioFocus.Duck).createSlot(0u)
    val audio = shadowOf(context.getSystemService(Context.AUDIO_SERVICE) as AudioManager)

    slot.load("k", route); idle(); advanceFocus(1000)

    assertEquals(emptyList<String>(), focusPort.calls.toList())

    slot.play(); settleAudioFocus()

    TestSignal.await("link focus", 5_000) { focusPort.calls.contains("request GAIN MEDIA") }
    assertEquals(listOf("request GAIN MEDIA"), focusPort.calls.toList())
    assertNull(audio.lastAudioFocusRequest)
  }

  @Test
  fun duckFollowsMutePauseAndUnloadOfTheSlot() {
    val slot = factory(startMuted = false, focus = MediaAudioFocus.Duck).createSlot(0u)
    slot.load("k", route); slot.play(); idle()

    slot.mute(); idle(); advanceFocus(300)

    TestSignal.await("abandon muted", 5_000) { focusPort.calls.size >= 2 }
    assertEquals(listOf("request GAIN MEDIA", "abandon"), focusPort.calls.toList())

    slot.unmute(); idle()

    TestSignal.await("focus unmuted", 5_000) { focusPort.calls.size >= 3 }
    assertEquals(listOf("request GAIN MEDIA", "abandon", "request GAIN MEDIA"), focusPort.calls.toList())

    slot.pause(); idle(); advanceFocus(300)

    TestSignal.await("abandon paused", 5_000) { focusPort.calls.size >= 4 }
    assertEquals(listOf("request GAIN MEDIA", "abandon", "request GAIN MEDIA", "abandon"), focusPort.calls.toList())

    slot.play(); idle(); slot.unload(); idle(); advanceFocus(300)

    TestSignal.await("abandon unloaded", 5_000) { focusPort.calls.size >= 6 }
    assertEquals(listOf("request GAIN MEDIA", "abandon", "request GAIN MEDIA", "abandon", "request GAIN MEDIA", "abandon"), focusPort.calls.toList())
  }

  @Test
  fun aMutedSlotStartedBySilentNeverTakesTheFocus() {
    val slot = factory(startMuted = true, focus = MediaAudioFocus.Duck).createSlot(0u)

    slot.load("k", route); slot.play(); idle(); advanceFocus(1000)

    assertEquals(emptyList<String>(), focusPort.calls.toList())
  }

  @Test
  fun aSlotThatIsReleasedGivesItsFocusBack() {
    val factory = factory(startMuted = false, focus = MediaAudioFocus.Duck)
    val slot = factory.createSlot(0u)
    slot.load("k", route); slot.play(); idle()

    slot.disposing(); idle(); advanceFocus(300)

    TestSignal.await("abandon disposed", 5_000) { focusPort.calls.size >= 2 }
    assertEquals(listOf("request GAIN MEDIA", "abandon"), focusPort.calls.toList())
  }

  @Test
  fun aDebugSlotStartsMutedAndAReleaseSlotStartsAtFullVolume() {
    listOf(true to 0f, false to 1f).forEach { (startMuted, volume) ->
      val slot = factory(startMuted = startMuted).createSlot(0u)
      slot.load("k", route); idle()

      assertEquals(volume, slot.player!!.volume, 0f)
    }
  }
}
