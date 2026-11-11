package com.noma4i.medialayer.ui

import com.noma4i.medialayer.testing.TestFetcher
import coil3.request.allowHardware
import kotlinx.coroutines.CompletableDeferred
import android.content.Context
import android.graphics.Bitmap
import android.graphics.Color
import androidx.compose.foundation.layout.size
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.CacheStrategy
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.OpenConfig
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.DeviceMemory
import com.noma4i.medialayer.defaultLimits
import com.noma4i.medialayer.defaultPoolSettings
import com.noma4i.medialayer.ToneListener
import com.noma4i.medialayer.ToneSource
import java.nio.file.Files
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

private data class Poster(override val url: String) : MediaSource {
  override val kind: Kind get() = Kind.IMAGE
  override val headers: Map<String, String> get() = emptyMap()
  override val size: Pair<Int, Int> get() = 24 to 24
}

private fun solid(width: Int, height: Int): Bitmap =
  Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888).apply { eraseColor(Color.rgb(200, 20, 40)) }

private fun item(url: String, cover: String? = null) = MediaItem(Kind.VIDEO, url, emptyMap(), cover, null, strategy = CacheStrategy.WINDOW)

@RunWith(MediaLayerTestRunner::class)
class ToneFramesTest {
  @get:Rule val rule = createComposeRule()

  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val layer = RecordingLayer()
  private val loader by lazy { mediaImageLoader(context, ImageBytes { _, _ -> com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK) }).newBuilder().allowHardware(false).build() }
  private val slots = ExoPlayerSlotFactory(context, startMuted = true).onMainLooper()

  @Test
  fun inv_tone_13_late_subscription_decodes_a_small_software_image_once() {
    val reads = AtomicInteger()
    val changes = AtomicInteger()
    val decodedBytes = AtomicInteger()
    val decodes = AtomicInteger()
    val urls = java.util.concurrent.CopyOnWriteArrayList<String>()
    val bytes = pngBytes(1080, 1920)
    val imageLoader = mediaImageLoader(context, ImageBytes { url, _ -> urls.add(url); reads.incrementAndGet(); com.noma4i.medialayer.ImageFetchResult(bytes, com.noma4i.medialayer.ImageOrigin.NETWORK) }).newBuilder()
      .eventListener(object : coil3.EventListener() {
        override fun decodeEnd(request: coil3.request.ImageRequest, decoder: coil3.decode.Decoder, options: coil3.request.Options, result: coil3.decode.DecodeResult?) {
          val bitmap = (result?.image as? coil3.BitmapImage)?.bitmap ?: return
          assertTrue(!request.allowHardware)
          assertTrue(bitmap.width <= 64 && bitmap.height <= 64)
          decodedBytes.addAndGet(bitmap.allocationByteCount)
          decodes.incrementAndGet()
        }
      }).build()
    val media = MediaLayer.open(OpenConfig(
      dir = Files.createTempDirectory("tone-demand").toString(),
      fetcher = TestFetcher(),
      settings = SharedPreferencesSettings(context, "tone-demand"), slots = null,
      device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings()
    ))
    val sources = listOf(
      MediaItem(Kind.IMAGE, "https://o.test/late.png", emptyMap(), null, null, strategy = CacheStrategy.WINDOW),
      item("https://o.test/video.mp4", "https://o.test/cover.png")
    )
    try {
      media.setTonePreparation(ToneFrames(media, context, imageLoader))
      assertEquals(0, reads.get())
      assertEquals(0, decodes.get())
      assertEquals(0, decodedBytes.get())
      sources.forEachIndexed { index, source ->
        assertTrue(!runBlocking { media.tonePixelsNeeded(source) })
        val subscription = media.subscribeTone(source, 0.15f, object : ToneListener {
          override fun changed(tone: com.noma4i.medialayer.MediaTone?) { if (tone?.source == ToneSource.PIXELS) changes.incrementAndGet() }
          override fun failed(error: com.noma4i.medialayer.MediaException) = Unit
        })
        try {
          rule.waitUntil(10_000) { org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle(); changes.get() == index + 1 }
          assertEquals(ToneSource.PIXELS, runBlocking { media.tone(source, 0.15f) }?.source)
          assertEquals(index + 1, reads.get())
          assertEquals(index + 1, decodes.get())
          assertTrue(decodedBytes.get() in 1..(16384 * (index + 1)))
          println("tone_key=${source.url} preparations=1 decoded_bytes=${decodedBytes.get() / (index + 1)} tone_updates=1")
          assertTrue(!runBlocking { media.tonePixelsNeeded(source) })
          media.itemState(source.url, com.noma4i.medialayer.MediaState.Ready)
          assertEquals(index + 1, reads.get())
        } finally { subscription.close() }
      }
      assertEquals(listOf("https://o.test/late.png", "https://o.test/cover.png"), urls.toList())
    } finally { imageLoader.shutdown(); media.close() }
  }

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
    val source = item("https://o.test/a.png")
    var main = true
    layer.needsTone = true
    val preparation = ToneFrames(layer, context, loader) {
      main = android.os.Looper.getMainLooper().isCurrentThread
      solid(300, 200)
    }
    val pixels = preparation.prepare(source)!!
    assertEquals(64u to 43u, pixels.width to pixels.height)
    assertEquals(64 * 43 * 4, pixels.rgba.size)
    assertTrue(!main)
  }

  @Test
  fun inv_native_25_aFileUrlToneIsReadThroughFetchImageExactlyOnce() = runBlocking {
    layer.needsTone = true
    layer.image = pngBytes()
    val imageLoader = mediaImageLoader(context, ImageBytes { url, headers -> layer.fetchImage(url, headers) }).newBuilder().allowHardware(false).build()
    val url = "file:///tmp/${java.util.UUID.randomUUID()}.png"

    val pixels = ToneFrames(layer, context, imageLoader).prepare(MediaItem(Kind.IMAGE, url, emptyMap(), null, null, strategy = CacheStrategy.WINDOW))
    imageLoader.shutdown()

    assertEquals(listOf("fetchImage:$url"), layer.calls.filter { it.startsWith("fetchImage:") })
    assertEquals(64 * 64 * 4, pixels?.rgba?.size)
    assertEquals(0, layer.mainThreadFfi.count { it == "cacheKey" })
  }

  @Test
  fun inv_tone_12_a_failed_decode_submits_nothing() = runBlocking {
    layer.needsTone = true
    val preparation = ToneFrames(layer, context, loader) { null }
    assertEquals(null, preparation.prepare(item("https://o.test/v.mp4")))
  }

  @Test
  fun inv_tone_12_a_video_frame_is_prepared_only_after_demand() = runBlocking {
    var frames = 0
    val preparation = ToneFrames(layer, context, loader) { frames++; solid(90, 160) }
    assertEquals(null, preparation.prepare(item("https://o.test/v.mp4")))
    assertEquals(0, frames)
    layer.needsTone = true
    val pixels = preparation.prepare(item("https://o.test/v.mp4"))!!
    assertEquals(36u to 64u, pixels.width to pixels.height)
    assertEquals(1, frames)
  }

  @Test
  fun inv_tone_13_image_loading_installs_preparation_without_requesting_pixels() {
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) { MediaImage(Poster("https://o.test/p.png"), Modifier.size(24.dp)) }
    }

    rule.waitUntil(10_000) { org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle(); layer.calls.contains("itemState:https://o.test/p.png:Ready") }
    assertTrue(layer.preparedTones != null)
    assertEquals(0, layer.toneChecks.get())
  }

}
