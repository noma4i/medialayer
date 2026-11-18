package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.Bitmap
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
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
import java.util.concurrent.Executor
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private val gifPage = object : MediaSource {
  override val url = "https://o.test/chat.gif"
  override val kind = Kind.IMAGE
  override val headers = emptyMap<String, String>()
  override val size = 24 to 24
}

@RunWith(MediaLayerTestRunner::class)
class RatedImageCompositionTest {
  @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val looper = shadowOf(android.os.Looper.getMainLooper())
  private val layer = RecordingLayer()
  private val fetches = AtomicInteger()
  private val animation = RatedAnimation(AnimationTimeline(intArrayOf(100, 100)), { _, _ -> }, bitmap(), bitmap(), 1.0, Repeats.Forever, executor = Executor { it.run() })
  private val loader = ImageLoader.Builder(context).components {
    add(Fetcher.Factory<MediaImageData> { _, _, _ ->
      Fetcher {
        fetches.incrementAndGet()
        ImageFetchResult(animation.asImage(false), false, DataSource.NETWORK)
      }
    })
  }.build()

  private fun bitmap(): Bitmap = Bitmap.createBitmap(4, 4, Bitmap.Config.ARGB_8888)

  private fun until(condition: () -> Boolean) {
    rule.waitForIdle()
    rule.waitUntil(COMPOSE_WAIT_MS) { looper.idle(); condition() }
  }

  @Test
  fun inv_image_77_aNewPlaybackRateReachesThePlayingImageWithoutLoadingItAgain() {
    var rate by mutableStateOf(0.5)
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader) { MediaImage(gifPage, Modifier.size(50.dp), playbackRate = rate) }
    }
    until { animation.isRunning && animation.rate == 0.5 }

    rate = 2.0
    until { animation.rate == 2.0 }
    rate = 0.25
    until { animation.rate == 0.25 }

    assertEquals("a change among rates other than 1 loads the image once", 1, fetches.get())
  }
}
