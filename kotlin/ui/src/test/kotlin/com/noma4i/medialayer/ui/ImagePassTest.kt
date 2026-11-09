package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.drawable.Drawable
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.size
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import coil3.ImageLoader
import coil3.asImage
import coil3.decode.DataSource
import coil3.fetch.Fetcher
import coil3.fetch.ImageFetchResult
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaState
import java.util.Collections
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private val passPage = object : MediaSource {
  override val url = "https://o.test/pass.jpg"
  override val kind = Kind.IMAGE
  override val headers = emptyMap<String, String>()
  override val size = 24 to 24
}

private class DrawProbe(private val log: MutableList<String>) : Drawable() {
  private var seen = false
  private fun seenByTheTree() { if (!seen) { seen = true; log.add("image") } }
  override fun draw(canvas: Canvas) { seenByTheTree() }
  override fun setAlpha(alpha: Int) {}
  override fun setColorFilter(colorFilter: android.graphics.ColorFilter?) {}
  @Deprecated("Deprecated in Java") override fun getOpacity() = android.graphics.PixelFormat.OPAQUE
  override fun getIntrinsicWidth() = 24
  override fun getIntrinsicHeight() = 24
}

@RunWith(MediaLayerTestRunner::class)
class ImagePassTest {
  @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val looper = shadowOf(android.os.Looper.getMainLooper())
  private val layer = RecordingLayer()
  private val log: MutableList<String> = Collections.synchronizedList(mutableListOf())

  private fun show(fetch: suspend () -> ImageFetchResult) {
    val loader = ImageLoader.Builder(context).components {
      add(Fetcher.Factory<MediaImageData> { _, _, _ -> Fetcher { fetch() } })
    }.build()
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader) {
        MediaImage(passPage, Modifier.size(50.dp)) { state -> log.add("content ${state::class.simpleName}") }
      }
    }
  }

  private val torn = mutableListOf<String>()

  private fun framesUntil(done: () -> Boolean) {
    rule.mainClock.autoAdvance = false
    val end = System.nanoTime() + 5_000_000_000L
    while (!done()) {
      check(System.nanoTime() < end) { "never held, events: ${log.toList()}" }
      looper.idle()
      rule.mainClock.advanceTimeByFrame()
      rule.runOnUiThread { rule.activity.window.decorView.draw(Canvas(Bitmap.createBitmap(64, 64, Bitmap.Config.ARGB_8888))) }
      val events = log.toList()
      if ("image" in events && events.lastOrNull { it.startsWith("content") } != "content Ready") torn.add("a frame ended with the image and ${events.lastOrNull { it.startsWith("content") }}: $events")
    }
  }

  @Test
  fun inv_image_80_the_state_given_to_content_is_ready_in_the_frame_that_draws_the_image() {
    show { ImageFetchResult(DrawProbe(log).asImage(false), false, DataSource.NETWORK) }

    framesUntil { log.contains("image") }

    assertTrue(torn.toString(), torn.isEmpty())
  }

  @Test
  fun inv_image_81_a_source_that_never_loads_keeps_the_loading_state_and_draws_no_image() {
    val never = CompletableDeferred<ImageFetchResult>()
    show { never.await() }

    framesUntil { log.any { it == "content Loading" || it == "content Idle" } }

    assertFalse(log.toList().toString(), log.contains("image") || log.contains("content Ready"))
  }

  @Test
  fun inv_image_82_a_failure_wins_and_no_image_is_drawn() {
    show { throw java.io.IOException("offline") }

    framesUntil { log.any { it == "content Failed" } }

    assertTrue(log.toList().toString(), !log.contains("image") && !log.contains("content Ready"))
  }
}
