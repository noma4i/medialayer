package com.noma4i.medialayer.ui

import android.app.Application
import android.content.Context
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.OpenConfig
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private data class ConnectClip(override val url: String = "https://o.test/connect.mp4", override val kind: Kind = Kind.VIDEO) : MediaSource {
  override val headers: Map<String, String> get() = emptyMap()
}

private class Seen {
  @Volatile var layer: MediaLayer? = null
}

@Composable
private fun Probe(seen: Seen) {
  val layer = currentMediaLayer()
  SideEffect { seen.layer = layer }
}

@RunWith(MediaLayerTestRunner::class)
class ConnectWithoutAppMachineTest {
  @get:Rule val rule = createComposeRule()

  private val application = ApplicationProvider.getApplicationContext<Application>()
  private val looper = shadowOf(android.os.Looper.getMainLooper())
  private val slots = ExoPlayerSlotFactory(application, startMuted = true).onMainLooper()
  private val loader by lazy { mediaImageLoader(application as Context, ImageBytes { _, _ -> error("unused") }) }

  private fun visibles(layer: RecordingLayer) = layer.calls.filter { it.startsWith("visible:https://o.test/connect.mp4:") }

  @Test
  fun inv_android_13_open_without_fetcher_and_settings_uses_okhttp_and_shared_preferences() {
    var config: OpenConfig? = null
    val lifecycle = LifecycleRegistry(object : LifecycleOwner { override val lifecycle: Lifecycle get() = error("unused") })
    val layer = RecordingLayer()

    openMediaLayer(application, lifecycle = lifecycle) { config = it; layer }.blocking()

    assertTrue(config!!.fetcher is OkHttpFetcher)
    assertTrue(config!!.settings is SharedPreferencesSettings)
  }

  @Test
  fun inv_android_14_a_media_view_declares_no_place_before_the_layer_is_ready_and_mounts_itself_after() {
    val layer = RecordingLayer()
    val opened = CompletableDeferred<MediaLayer>()
    var drawn = false
    rule.setContent { MediaLayerProvider(opened, loader, slots.players) { SideEffect { drawn = true }; MediaVideo(ConnectClip(), modifier = Modifier.size(50.dp)) } }
    rule.waitForIdle()
    looper.idleFor(java.time.Duration.ofMillis(1000))
    assertTrue(drawn)
    assertEquals(emptyList<String>(), visibles(layer))

    opened.complete(layer)
    rule.waitUntil(5000) { looper.idle(); visibles(layer).isNotEmpty() }
  }

  @Test
  fun inv_android_15_an_opening_failure_shows_in_media_failure() {
    Media.failure = null
    val opened = CompletableDeferred<MediaLayer>()
    rule.setContent { MediaLayerProvider(opened, loader, slots.players) { } }

    opened.completeExceptionally(MediaException.Fetch("connect boom"))
    rule.waitUntil(5000) { looper.idle(); Media.failure != null }

    assertEquals("connect boom", (Media.failure as MediaException.Fetch).detail)
    Media.failure = null
  }

  @Test
  fun inv_android_18_any_opening_exception_shows_in_media_failure_as_unexpected() {
    Media.failure = null
    val opened = CompletableDeferred<MediaLayer>()
    rule.setContent { MediaLayerProvider(opened, loader, slots.players) { } }

    opened.completeExceptionally(IllegalStateException("native library missing"))
    rule.waitUntil(5000) { looper.idle(); Media.failure != null }

    assertTrue((Media.failure as MediaException.Unexpected).toString().contains("native library missing"))
    Media.failure = null
  }

  @Test
  fun inv_android_16_a_view_outside_the_provider_takes_the_process_context_and_the_environment_wins_and_closing_the_layer_clears_it() {
    val first = RecordingLayer()
    val second = RecordingLayer()
    var showFirst by mutableStateOf(true)
    var showSecond by mutableStateOf(true)
    val insideFirst = Seen()
    val outside = Seen()
    rule.setContent {
      Column {
        if (showFirst) MediaLayerProvider(CompletableDeferred(first), loader, slots.players) { Probe(insideFirst) }
        if (showSecond) MediaLayerProvider(CompletableDeferred(second), loader, slots.players) { }
        Probe(outside)
      }
    }
    rule.waitForIdle()
    rule.waitUntil(5000) { looper.idle(); outside.layer === second }
    assertSame(first, insideFirst.layer)

    showFirst = false
    rule.waitForIdle()
    assertSame(second, outside.layer)

    showSecond = false
    rule.waitForIdle()
    assertSame("the process layer outlives the screen", second, outside.layer)

    second.destroy()
    rule.waitForIdle()
    assertNull(outside.layer)
  }

  @Test
  fun inv_android_17_layer_access_is_null_before_ready_and_the_layer_after() {
    val layer = RecordingLayer()
    val opened = CompletableDeferred<MediaLayer>()
    val inside = Seen()
    val outside = Seen()
    rule.setContent {
      Column {
        MediaLayerProvider(opened, loader, slots.players) { Probe(inside) }
        Probe(outside)
      }
    }
    rule.waitForIdle()
    assertNull(inside.layer)
    assertNull(outside.layer)

    opened.complete(layer)
    rule.waitUntil(5000) { looper.idle(); inside.layer === layer && outside.layer === layer }
  }
}
