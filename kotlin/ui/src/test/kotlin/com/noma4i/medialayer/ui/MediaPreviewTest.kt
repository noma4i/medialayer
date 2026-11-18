package com.noma4i.medialayer.ui

import android.content.Context
import androidx.compose.foundation.layout.size
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import coil3.request.allowHardware
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSource
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private data class PreviewPhoto(override val url: String) : MediaSource {
  override val kind: Kind get() = Kind.IMAGE
  override val headers: Map<String, String> get() = emptyMap()
}

@RunWith(MediaLayerTestRunner::class)
class MediaPreviewTest {
  @get:Rule val rule = createComposeRule()

  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val layer = RecordingLayer()
  private val slots = ExoPlayerSlotFactory(context, startMuted = true).onMainLooper()
  private val looper = shadowOf(android.os.Looper.getMainLooper())
  private val gate = CompletableDeferred<Unit>()
  private val asked = CopyOnWriteArrayList<String>()
  private val main = PreviewPhoto("https://o.test/uploaded.png")
  private val local = PreviewPhoto("file:///cache/local.png")

  private val loader by lazy {
    mediaImageLoader(context, ImageBytes { url, _ ->
      asked.add(url)
      if (url == main.url) gate.await()
      com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK)
    }, stats = layer.imageStats()).newBuilder().allowHardware(false).build()
  }

  private fun previews() = rule.onAllNodesWithTag(PREVIEW_TAG)

  @Test
  fun inv_image_48_thePreviewShowsUntilTheFirstFrameOfTheMainImageAndTakesNoPlaceOfItsOwn() {
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) { MediaImage(main, Modifier.size(50.dp), preview = local) }
    }
    rule.waitUntil(COMPOSE_WAIT_MS) { looper.idle(); layer.imageStats().snapshot().decodes == 1uL }
    previews().assertCountEquals(1)
    assertEquals(listOf(local.url, main.url), asked.toList().sortedBy { if (it == local.url) 0 else 1 })

    gate.complete(Unit)
    rule.waitUntil(COMPOSE_WAIT_MS) { looper.idle(); layer.calls.any { it == "itemState:${main.url}:Ready" } }
    rule.waitUntil(COMPOSE_WAIT_MS) { looper.idle(); previews().fetchSemanticsNodes().isEmpty() }

    assertTrue(layer.calls.toString(), layer.calls.none { local.url in it })
    drainFfi()
    assertEquals(2uL, layer.imageStats().snapshot().decodes)
    assertEquals(0, layer.mainThreadFfi.count { it == "cacheKey" })
  }
}
