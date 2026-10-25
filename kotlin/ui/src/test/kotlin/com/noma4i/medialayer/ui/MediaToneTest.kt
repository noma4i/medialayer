package com.noma4i.medialayer.ui

import android.content.Context
import androidx.compose.runtime.State
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaTone
import com.noma4i.medialayer.Rgb
import com.noma4i.medialayer.Swatch
import com.noma4i.medialayer.ToneSource
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private data class ToneSubject(override val url: String) : MediaSource {
  override val kind: Kind get() = Kind.IMAGE
  override val headers: Map<String, String> get() = emptyMap()
}

private fun toneOf(level: Int, band: Float = 0.15f): MediaTone {
  val swatch = Swatch(Rgb(level.toUByte(), level.toUByte(), level.toUByte()), level / 255.0)
  return MediaTone(ToneSource.PIXELS, 1u, 1u, emptyList(), band, swatch, swatch, swatch, emptyList())
}

@RunWith(MediaLayerTestRunner::class)
class MediaToneTest {
  @get:Rule val rule = createComposeRule()

  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val layer = RecordingLayer()
  private val loader by lazy { mediaImageLoader(context, ImageBytes { _, _ -> com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK) }) }
  private val slots = ExoPlayerSlotFactory(context, startMuted = true).onMainLooper()
  private val looper = shadowOf(android.os.Looper.getMainLooper())

  private var url by mutableStateOf<String?>("https://o.test/a.jpg")
  private var band by mutableStateOf(0.15f)
  private var mounted by mutableStateOf(true)
  private var shown: State<MediaToneState>? = null
  private val current: MediaToneState get() = shown!!.value

  private fun mount() {
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
        if (mounted) shown = rememberMediaTone(url?.let { ToneSubject(it) }, band)
      }
    }
  }

  private fun until(condition: () -> Boolean) = rule.waitUntil(5000) { looper.idle(); condition() }

  @Test
  fun inv_tone_20_mounting_subscribes_once_and_unmounting_cancels() {
    mount()
    until { layer.toneWatches.size == 1 }
    val watch = layer.toneWatches[0]
    assertEquals("https://o.test/a.jpg", watch.item.url)
    assertEquals(0.15f, watch.band)
    watch.listener.changed(toneOf(40))
    until { current.tone == toneOf(40) }
    assertFalse(watch.closed.get())

    mounted = false
    until { watch.closed.get() }
    assertEquals(1, layer.toneWatches.size)
  }

  @Test
  fun inv_tone_20_a_new_source_replaces_the_subscription_and_a_stale_callback_is_ignored() {
    mount()
    until { layer.toneWatches.size == 1 }
    layer.toneWatches[0].listener.changed(toneOf(40))
    until { current.tone == toneOf(40) }

    url = "https://o.test/b.jpg"
    until { layer.toneWatches.size == 2 && layer.toneWatches[0].closed.get() }
    assertEquals("https://o.test/b.jpg", layer.toneWatches[1].item.url)
    assertNull(current.tone)

    layer.toneWatches[0].listener.changed(toneOf(200))
    layer.toneWatches[1].listener.changed(toneOf(90))
    until { current.tone == toneOf(90) }
    looper.idle()
    assertEquals(toneOf(90), current.tone)
  }

  @Test
  fun inv_tone_20_a_null_source_cancels_and_clears_and_a_band_change_keeps_the_value() {
    mount()
    until { layer.toneWatches.size == 1 }
    layer.toneWatches[0].listener.changed(toneOf(40))
    until { current.tone == toneOf(40) }

    band = 0.5f
    until { layer.toneWatches.size == 2 && layer.toneWatches[0].closed.get() }
    assertEquals(0.5f, layer.toneWatches[1].band)
    assertEquals(toneOf(40), current.tone)

    url = null
    until { layer.toneWatches[1].closed.get() && current.tone == null }
    assertEquals(2, layer.toneWatches.size)
  }

  @Test
  fun inv_tone_20_the_platform_keeps_no_table_of_urls_so_a_returning_source_asks_the_core_again() {
    mount()
    until { layer.toneWatches.size == 1 }
    layer.toneWatches[0].listener.changed(toneOf(40))
    until { current.tone == toneOf(40) }

    url = "https://o.test/b.jpg"
    until { layer.toneWatches.size == 2 }
    url = "https://o.test/a.jpg"
    until { layer.toneWatches.size == 3 }
    assertNull(current.tone)
    assertTrue(layer.mainThreadFfi.none { it == "subscribeTone" })
  }
}
