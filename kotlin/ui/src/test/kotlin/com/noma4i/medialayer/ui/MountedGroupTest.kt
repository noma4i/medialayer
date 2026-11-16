package com.noma4i.medialayer.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import com.noma4i.medialayer.testing.TestFetcher
import com.noma4i.medialayer.CreatedPlayer
import com.noma4i.medialayer.SlotCall
import android.content.Context
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.*
import java.nio.file.Files
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private const val CLIP = "file:///clip.mp4"

private data class ClipSource(override val url: String, override val kind: Kind) : MediaSource {
  override val headers: Map<String, String> get() = emptyMap()
}

private class TransportSlot : PlayerSlot, PlayerSlotFactory {
  val log = CopyOnWriteArrayList<String>()
  val transport: String? get() = log.lastOrNull { it == "play" || it == "pause" }
  override suspend fun create(slot: UInt): CreatedPlayer = CreatedPlayer(this)
  override suspend fun apply(command: SlotCall) {
    when (command) {
      SlotCall.Play -> log.add("play")
      SlotCall.Pause -> log.add("pause")
      is SlotCall.Rate -> log.add("rate")
      else -> {}
    }
  }
  override suspend fun dispose() {}
  override fun sample() = SlotSample(0u, null, false, false, false)
}

@RunWith(MediaLayerTestRunner::class)
class MountedGroupTest {
  @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()
  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val slot = TransportSlot()
  private val loader = mediaImageLoader(context, ImageBytes { _, _ -> com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK) })
  private val layer = MediaLayer.open(OpenConfig(
      audioFocus = com.noma4i.medialayer.AudioFocus.EXCLUSIVE,
    dir = Files.createTempDirectory("mounted-group").toString(),
    fetcher = TestFetcher(),
    settings = SharedPreferencesSettings(context, "mounted-group"), slots = slot,
    device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings()
  ))

  private val looper = shadowOf(Looper.getMainLooper())

  private fun waitFor(transport: String) = rule.waitUntil(10_000) { looper.idle(); slot.transport == transport }

  @Test
  fun inv_session_15_a_mounted_video_follows_a_change_of_its_group_only() {
    var group by mutableStateOf("ring")
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, null) {
        MediaVideo(ClipSource(CLIP, Kind.VIDEO), group = group, modifier = Modifier.size(80.dp))
      }
    }
    waitFor("play")
    val hold = layer.suspendPlayback(listOf("system"))
    waitFor("pause")
    rule.runOnIdle { group = "system" }
    waitFor("play")
    rule.runOnIdle { group = "ring" }
    waitFor("pause")
    assertEquals("pause", slot.transport)
    hold.release()
  }

  @Test
  fun inv_session_15_a_mounted_audio_follows_a_change_of_its_group_only() {
    var group by mutableStateOf("ring")
    var control: AudioControl? = null
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, null) {
        MediaAudio(ClipSource(CLIP, Kind.AUDIO), group = group) { control = it; Box(Modifier.size(80.dp)) }
      }
    }
    rule.waitUntil(10_000) { control != null }
    rule.runOnIdle { control!!.play() }
    waitFor("play")
    val hold = layer.suspendPlayback(listOf("system"))
    waitFor("pause")
    rule.runOnIdle { group = "system" }
    waitFor("play")
    rule.runOnIdle { group = "ring" }
    waitFor("pause")
    assertEquals("pause", slot.transport)
    hold.release()
  }

  private fun mountToggledAudio(group: () -> String): AudioControl {
    var control: AudioControl? = null
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, null) {
        MediaAudio(ClipSource(CLIP, Kind.AUDIO), group = group()) { control = it; Box(Modifier.size(80.dp)) }
      }
    }
    rule.waitUntil(10_000) { control != null }
    rule.runOnIdle { control!!.play() }
    waitFor("play")
    return control!!
  }

  @Test
  fun inv_session_16_a_toggled_audio_stays_playing_when_another_view_of_its_key_comes_and_goes() {
    val audio = mountToggledAudio { "ring" }
    val other = layer.openPlace(MediaItem(Kind.VIDEO, CLIP, emptyMap(), null, null, strategy = CacheStrategy.WINDOW))
    other.update(placeInput(playing = true, group = "chat"))
    other.release()
    rule.runOnIdle { audio.rate = 2f }
    rule.waitUntil(10_000) { looper.idle(); slot.log.last() == "rate" }
    assertEquals("play", slot.transport)
  }

  @Test
  fun inv_session_14_a_key_with_several_playing_owners_is_allowed_by_any_of_their_groups() {
    val item = MediaItem(Kind.VIDEO, CLIP, emptyMap(), null, null, strategy = CacheStrategy.WINDOW)
    val ring = layer.openPlace(item)
    val system = layer.openPlace(item)
    ring.update(placeInput(visible = true, playing = true, group = "ring"))
    system.update(placeInput(playing = true, group = "system"))
    waitFor("play")

    val allowing = layer.suspendPlayback(listOf("system"))
    layer.setRate(CLIP, 2f)
    rule.waitUntil(10_000) { slot.log.last() == "rate" }
    assertEquals("play", slot.transport)

    val blocking = layer.suspendPlayback(listOf("ring"))
    waitFor("pause")
    blocking.release()
    waitFor("play")
    allowing.release()
  }
}
