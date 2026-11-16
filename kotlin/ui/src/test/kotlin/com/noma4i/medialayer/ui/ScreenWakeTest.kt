package com.noma4i.medialayer.ui

import android.os.Looper
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaState
import com.noma4i.medialayer.NoHandle
import com.noma4i.medialayer.PlaceInput
import com.noma4i.medialayer.PlaybackListener
import com.noma4i.medialayer.PlaybackState
import com.noma4i.medialayer.Subscription
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertFalse
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private data class WakeClip(override val url: String) : MediaSource {
  override val kind: Kind get() = Kind.AUDIO
  override val headers: Map<String, String> get() = emptyMap()
}

/** A place of the core whose playback subscription the test drives, like the core pushes `PlaybackState`. */
private class DrivenPlace : com.noma4i.medialayer.MediaPlace(NoHandle) {
  @Volatile private var listener: PlaybackListener? = null
  @Volatile var released = false

  val subscribed: Boolean get() = listener != null

  fun push(playing: Boolean) { listener?.changed(PlaybackState(playing, playing, null, null, 0uL)) }

  override fun subscribePlayback(listener: PlaybackListener): Subscription {
    this.listener = listener
    TestSignal.fire()
    return Subscription(NoHandle)
  }
  override fun update(input: PlaceInput) = Unit
  override fun seek(fraction: Double) = Unit
  override fun itemState(state: MediaState) = Unit
  override fun release() { released = true; TestSignal.fire() }
  override fun close() = Unit
}

@RunWith(MediaLayerTestRunner::class)
class ScreenWakeTest {
  @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()
  private val looper = shadowOf(Looper.getMainLooper())
  private val place = DrivenPlace()
  private val layer = RecordingLayer().also { it.placeFactory = { place } }
  private var shown by mutableStateOf(true)

  private val screenKeptOn: Boolean
    get() = rule.activity.window.attributes.flags and WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON != 0

  private fun until(condition: () -> Boolean) = rule.waitUntil(5_000) { looper.idle(); condition() }

  private fun mount() {
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer)) {
        if (shown) MediaAudio(WakeClip("file:///wake.m4a")) { Box(Modifier.size(40.dp)) }
      }
    }
    until { place.subscribed }
  }

  @Test
  fun inv_api_60_a_playing_place_keeps_the_screen_on() {
    mount()
    assertFalse(screenKeptOn)

    place.push(playing = true)
    until { screenKeptOn }

    place.push(playing = false)
    until { !screenKeptOn }
  }

  @Test
  fun inv_api_61_a_late_delivery_after_release_does_not_keep_the_screen_on() {
    mount()
    rule.runOnIdle { shown = false }
    until { place.released }

    place.push(playing = true)
    looper.idle()

    assertFalse(screenKeptOn)
  }
}
