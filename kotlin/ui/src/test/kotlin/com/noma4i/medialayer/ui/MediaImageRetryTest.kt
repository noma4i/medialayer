package com.noma4i.medialayer.ui

import android.content.Context
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import coil3.request.allowHardware
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaState
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private data class RetriedPhoto(override val url: String) : MediaSource {
  override val kind: Kind get() = Kind.IMAGE
  override val headers: Map<String, String> get() = emptyMap()
}

@RunWith(MediaLayerTestRunner::class)
class MediaImageRetryTest {
  @get:Rule val rule = createComposeRule()

  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val layer = RecordingLayer()
  private val slots = ExoPlayerSlotFactory(context, startMuted = true).onMainLooper()
  private val looper = shadowOf(android.os.Looper.getMainLooper())
  private val photo = RetriedPhoto("file:///cache/picked.png")
  private val fetches = AtomicInteger()

  private val loader by lazy {
    mediaImageLoader(context, ImageBytes { _, _ ->
      if (fetches.incrementAndGet() == 1) throw MediaException.Fetch("the file is not there yet")
      com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.CACHE)
    }, stats = layer.imageStats()).newBuilder().allowHardware(false).build()
  }

  @Test
  fun inv_image_83_a_failed_image_mounted_again_under_a_new_key_loads_the_same_source_again() {
    var attempt by mutableIntStateOf(0)
    var shown: MediaState = MediaState.Idle
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
        key(attempt) { MediaImage(photo, Modifier.size(50.dp)) { state -> shown = state } }
      }
    }
    rule.waitUntil(COMPOSE_WAIT_MS) { looper.idle(); shown is MediaState.Failed }

    attempt += 1
    rule.waitUntil(COMPOSE_WAIT_MS) { looper.idle(); shown == MediaState.Ready }

    assertEquals(2, fetches.get())
    drainFfi()
    assertEquals(1uL, layer.imageStats().snapshot().decodes)
  }

  @Test
  fun inv_api_59_retry_reloads_a_failed_place_without_remount() {
    var shown: MediaState = MediaState.Idle
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
        MediaImage(photo, Modifier.size(50.dp)) { state -> shown = state }
      }
    }
    rule.waitUntil(COMPOSE_WAIT_MS) { looper.idle(); shown is MediaState.Failed }

    runBlocking { Media.retry(photo) }
    rule.waitUntil(COMPOSE_WAIT_MS) { looper.idle(); shown == MediaState.Ready }

    assertEquals(2, fetches.get())
    assertEquals(listOf("retry:${photo.url}"), layer.calls.filter { it.startsWith("retry:") })
    assertEquals(1, layer.calls.count { it == "visible:${photo.url}:true" })
  }
}
