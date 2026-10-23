package com.noma4i.medialayer.ui

import android.content.Context
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.getValue
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
class MountedGroupTest {
  @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()
  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val slot = TransportSlot()
  private val loader = mediaImageLoader(context, ImageBytes { _, _ -> com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK) })
  private val layer = MediaLayer.open(OpenConfig(availableBytes = ULong.MAX_VALUE,
    dir = Files.createTempDirectory("mounted-group").toString(),
    fetcher = object : Fetcher {
      override suspend fun fetch(request: FetchRequest): FetchResponse = throw MediaException.Fetch("unused origin")
    },
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

  @Test
  fun inv_session_14_a_key_with_several_playing_owners_is_allowed_by_any_of_their_groups() {
    val claims = layer.claims
    val item = MediaItem(Kind.VIDEO, CLIP, emptyMap(), null, null, strategy = CacheStrategy.WINDOW)
    val ring = Any()
    val system = Any()
    claims.visible(ring, item, true)
    claims.setPlaying(ring, CLIP, true, "ring")
    claims.setPlaying(system, CLIP, true, "system")
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
