package com.noma4i.medialayer.ui

import android.content.Context
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.*
import java.nio.file.Files
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotSame
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private data class InputSource(override val url: String, override val kind: Kind) : MediaSource {
  override val headers: Map<String, String> get() = emptyMap()
}

private class InputSlot : PlayerSlot, PlayerSlotFactory {
  val log = CopyOnWriteArrayList<String>()
  val transport: String? get() = log.lastOrNull { it == "play" || it == "pause" }
  override fun create(slot: UInt): PlayerSlot = this
  override fun load(key: String, route: PlaybackRoute) {}
  override fun unload() {}
  override fun preroll() {}
  override fun play() { log.add("play") }
  override fun pause() { log.add("pause") }
  override fun mute() {}
  override fun unmute() {}
  override fun attach(viewToken: ULong) {}
  override fun detach() {}
  override fun seek(positionMs: ULong) {}
  override fun setRate(rate: Float) { log.add("rate") }
  override fun setReadahead(ms: ULong) {}
  override fun sample() = SlotSample(0u, null, false, false, false)
}

@RunWith(MediaLayerTestRunner::class)
class MediaAudioInputTest {
  @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()
  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val slot = InputSlot()
  private val loader = mediaImageLoader(context, ImageBytes { _, _ -> com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK) })
  private val layer = MediaLayer.open(OpenConfig(availableBytes = ULong.MAX_VALUE,
    dir = Files.createTempDirectory("audio-input").toString(),
    fetcher = object : Fetcher {
      override suspend fun fetch(request: FetchRequest): FetchResponse = throw MediaException.Fetch("unused origin")
    },
    settings = SharedPreferencesSettings(context, "audio-input"), slots = slot,
    device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings()
  ))

  private val looper = shadowOf(Looper.getMainLooper())
  private val seen = CopyOnWriteArrayList<AudioControl>()
  private var url by mutableStateOf("file:///first.mp4")
  private var playing by mutableStateOf(false)
  private var rate by mutableStateOf(1f)
  private var muted by mutableStateOf(false)
  private var group by mutableStateOf("ring")
  private var unrelated by mutableIntStateOf(0)

  private fun waitFor(transport: String) = rule.waitUntil(10_000) { looper.idle(); slot.transport == transport }

  private fun mount(): AudioControl {
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, null) {
        val tick = unrelated
        MediaAudio(InputSource(url, Kind.AUDIO), playing = playing, muted = muted, rate = rate, group = group) {
          seen.add(it)
          Box(Modifier.size(80.dp + tick.dp))
        }
      }
    }
    rule.waitUntil(10_000) { seen.isNotEmpty() }
    return seen.last()
  }

  @Test
  fun a_mounted_audio_follows_a_changing_playing_input_without_remount() {
    val first = mount()
    rule.runOnIdle { assertFalse(first.isPlaying) }
    assertTrue(slot.transport != "play")

    rule.runOnIdle { playing = true }
    waitFor("play")
    rule.runOnIdle { assertTrue(first.isPlaying) }

    rule.runOnIdle { playing = false }
    waitFor("pause")
    rule.runOnIdle { assertFalse(first.isPlaying) }
    seen.forEach { assertSame(first, it) }
  }

  @Test
  fun a_mounted_audio_starts_at_an_initial_true_playing_input() {
    playing = true
    val first = mount()
    waitFor("play")
    rule.runOnIdle { assertTrue(first.isPlaying) }
  }

  @Test
  fun a_local_toggle_survives_rerenders_that_leave_playing_unchanged() {
    val first = mount()
    rule.runOnIdle { first.toggle() }
    waitFor("play")

    rule.runOnIdle {
      unrelated += 1
      rate = 2f
      muted = true
      group = "system"
    }
    rule.waitUntil(10_000) { looper.idle(); slot.log.last() == "rate" }
    rule.waitForIdle()

    rule.runOnIdle { assertTrue(first.isPlaying) }
    assertEquals("play", slot.transport)
    assertFalse(slot.log.contains("pause"))
    seen.forEach { assertSame(first, it) }
  }

  @Test
  fun a_changed_source_starts_from_the_current_playing_input() {
    playing = true
    val first = mount()
    waitFor("play")

    rule.runOnIdle {
      url = "file:///second.mp4"
      playing = false
    }
    rule.waitUntil(10_000) { looper.idle(); seen.last() !== first }
    rule.runOnIdle { assertFalse(seen.last().isPlaying) }
    waitFor("pause")

    rule.runOnIdle {
      url = "file:///third.mp4"
      playing = true
    }
    rule.waitUntil(10_000) { looper.idle(); seen.last().isPlaying }
    assertNotSame(first, seen.last())
    waitFor("play")
  }
}
