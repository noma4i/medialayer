package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Color
import androidx.compose.foundation.layout.size
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.PlaybackRoute
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

private data class Poster(override val url: String) : MediaSource {
  override val kind: Kind get() = Kind.IMAGE
  override val headers: Map<String, String> get() = emptyMap()
  override val size: Pair<Int, Int> get() = 24 to 24
}

private data class Covered(override val url: String, override val cover: String?) : MediaSource {
  override val kind: Kind get() = Kind.VIDEO
  override val headers: Map<String, String> get() = emptyMap()
}

private fun solid(width: Int, height: Int): Bitmap =
  Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888).apply { eraseColor(Color.rgb(200, 20, 40)) }

private fun item(url: String, cover: String? = null) = MediaItem(Kind.VIDEO, url, emptyMap(), cover, null)

private val route = PlaybackRoute("http://127.0.0.1:1/v.mp4", emptyMap(), null, false, true)

@RunWith(RobolectricTestRunner::class)
class ToneFramesTest {
  @get:Rule val rule = createComposeRule()

  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val layer = RecordingLayer()
  private val loader by lazy { mediaImageLoader(context, ImageBytes { _, _ -> pngBytes() }).newBuilder().allowHardware(false).build() }
  private val slots = ExoPlayerSlotFactory(context, startMuted = true)

  @Test
  fun inv_tone_12_a_decoded_image_is_shrunk_to_64_on_the_long_side_keeping_its_colours() {
    listOf(Triple(200, 100, 64 to 32), Triple(100, 300, 21 to 64), Triple(30, 20, 30 to 20)).forEach { (width, height, expected) ->
      val shrunk = ToneFrames.pixels(solid(width, height))!!

      assertEquals(expected, shrunk.width to shrunk.height)
      assertEquals(expected.first * expected.second * 4, shrunk.rgba.size)
      val channels = (0..3).map { shrunk.rgba[it].toInt() and 0xFF }
      assertTrue(channels.toString(), channels.zip(listOf(200, 20, 40, 255)).all { (got, want) -> kotlin.math.abs(got - want) <= 2 })
    }
  }

  @Test
  fun inv_tone_12_the_pixels_reach_the_core_off_the_main_thread() = runBlocking {
    ToneFrames.submit(layer, item("https://o.test/a.png"), solid(300, 200))

    assertEquals(listOf(ToneCall("https://o.test/a.png", 64, 43, 64 * 43 * 4, false)), layer.tones.toList())
  }

  @Test
  fun inv_tone_12_a_failed_decode_submits_nothing() = runBlocking {
    ToneFrames.submit(layer, item("https://o.test/a.png"), null as Bitmap?)
    ToneFrames.submitFirstFrame(layer, item("https://o.test/v.mp4"), route) { null }

    assertTrue(layer.tones.isEmpty())
  }

  @Test
  fun inv_tone_12_a_video_without_a_cover_gives_its_first_frame_and_a_covered_one_does_not() = runBlocking {
    ToneFrames.submitFirstFrame(layer, item("https://o.test/v.mp4", cover = "https://o.test/c.jpg"), route) { solid(90, 160) }
    ToneFrames.submitFirstFrame(layer, item("https://o.test/v.mp4"), null) { solid(90, 160) }
    assertTrue(layer.tones.isEmpty())

    ToneFrames.submitFirstFrame(layer, item("https://o.test/v.mp4"), route) { solid(90, 160) }

    assertEquals(listOf(ToneCall("https://o.test/v.mp4", 36, 64, 36 * 64 * 4, false)), layer.tones.toList())
  }

  @Test
  fun inv_tone_12_a_loaded_image_reaches_the_core() {
    rule.setContent {
      MediaLayerProvider(layer, loader, slots.players) { MediaImage(Poster("https://o.test/p.png"), Modifier.size(24.dp)) }
    }

    rule.waitUntil(10_000) { org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle(); layer.tones.isNotEmpty() }
    assertEquals("https://o.test/p.png", layer.tones.first().url)
    assertTrue(layer.tones.all { !it.onMain && it.width <= 64 && it.height <= 64 })
  }

  @Test
  fun inv_tone_12_a_video_cover_gives_the_video_its_tone() {
    rule.setContent {
      MediaLayerProvider(layer, loader, slots.players) { MediaVideo(Covered("https://o.test/v.mp4", "https://o.test/c.jpg"), modifier = Modifier.size(24.dp)) }
    }

    rule.waitUntil(10_000) { org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle(); layer.tones.isNotEmpty() }
    assertEquals("https://o.test/v.mp4", layer.tones.first().url)
  }
}
