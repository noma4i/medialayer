package com.noma4i.medialayer.ui

import android.content.Context
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import coil3.request.allowHardware
import com.noma4i.medialayer.ImageFetchResult
import com.noma4i.medialayer.ImageOrigin
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSource
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

@RunWith(MediaLayerTestRunner::class)
class MediaImageInterimTest {
  @get:Rule val rule = createComposeRule()

  @Test
  fun inv_image_88_cachedPreviewShowsUntilTheMainImageIsReady() {
    val context = ApplicationProvider.getApplicationContext<Context>()
    val layer = RecordingLayer()
    val slots = ExoPlayerSlotFactory(context, startMuted = true).onMainLooper()
    val tile = object : MediaSource {
      override val url = "https://example.test/tile.png"
      override val kind = Kind.IMAGE
      override val headers = emptyMap<String, String>()
    }
    val main = object : MediaSource {
      override val url = "https://example.test/main.png"
      override val kind = Kind.IMAGE
      override val headers = emptyMap<String, String>()
    }
    val gate = CompletableDeferred<Unit>()
    val requests = AtomicInteger()
    val loader = mediaImageLoader(context, ImageBytes { url, _ ->
      if (url == main.url || requests.incrementAndGet() > 1) gate.await()
      ImageFetchResult(pngBytes(100, 100), ImageOrigin.NETWORK)
    }, stats = layer.imageStats()).newBuilder().allowHardware(false).build()
    val looper = shadowOf(android.os.Looper.getMainLooper())
    var showMain by mutableStateOf(false)

    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
        if (showMain) MediaImage(main, Modifier.size(50.dp), preview = tile)
        else MediaImage(tile, Modifier.size(20.dp))
      }
    }
    rule.waitUntil(COMPOSE_WAIT_MS) { looper.idle(); layer.calls.any { it == "itemState:${tile.url}:Ready" } }
    val decodes = layer.imageStats().snapshot().decodes
    assertEquals(1uL, decodes)

    rule.runOnIdle { showMain = true }
    rule.waitUntil(COMPOSE_WAIT_MS) { looper.idle(); requests.get() == 2 }
    rule.onAllNodesWithTag(PREVIEW_TAG).assertCountEquals(1)
    assertEquals(decodes, layer.imageStats().snapshot().decodes)

    gate.complete(Unit)
    rule.waitUntil(COMPOSE_WAIT_MS) { looper.idle(); layer.calls.any { it == "itemState:${main.url}:Ready" } }
    rule.waitUntil(COMPOSE_WAIT_MS) { looper.idle(); rule.onAllNodesWithTag(PREVIEW_TAG).fetchSemanticsNodes().isEmpty() }
    rule.onAllNodesWithTag(PREVIEW_TAG).assertCountEquals(0)
  }

  @Test
  fun inv_image_88_cachedPixelsShowUntilTheNewSizeIsDecoded() {
    val context = ApplicationProvider.getApplicationContext<Context>()
    val layer = RecordingLayer()
    val slots = ExoPlayerSlotFactory(context, startMuted = true).onMainLooper()
    val source = object : MediaSource {
      override val url = "https://example.test/image.png"
      override val kind = Kind.IMAGE
      override val headers = emptyMap<String, String>()
    }
    val gate = CompletableDeferred<Unit>()
    val requests = AtomicInteger()
    val loader = mediaImageLoader(context, ImageBytes { _, _ ->
      if (requests.incrementAndGet() > 1) gate.await()
      ImageFetchResult(pngBytes(100, 100), ImageOrigin.NETWORK)
    }, stats = layer.imageStats()).newBuilder().allowHardware(false).build()
    val looper = shadowOf(android.os.Looper.getMainLooper())
    var side by mutableStateOf(20)

    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
        key(side) { MediaImage(source, Modifier.size(side.dp)) }
      }
    }
    rule.waitUntil(COMPOSE_WAIT_MS) { looper.idle(); layer.calls.count { it == "itemState:${source.url}:Ready" } == 1 }
    val decodes = layer.imageStats().snapshot().decodes
    assertEquals(1uL, decodes)

    rule.runOnIdle { side = 50 }
    rule.waitUntil(COMPOSE_WAIT_MS) { looper.idle(); requests.get() == 2 }
    rule.waitUntil(COMPOSE_WAIT_MS) { looper.idle(); rule.onAllNodesWithTag(INTERIM_TAG).fetchSemanticsNodes().isNotEmpty() }
    rule.onAllNodesWithTag(INTERIM_TAG).assertCountEquals(1)
    assertEquals(decodes, layer.imageStats().snapshot().decodes)

    gate.complete(Unit)
    rule.waitUntil(COMPOSE_WAIT_MS) { looper.idle(); layer.calls.count { it == "itemState:${source.url}:Ready" } == 2 }
    rule.waitUntil(COMPOSE_WAIT_MS) { looper.idle(); rule.onAllNodesWithTag(INTERIM_TAG).fetchSemanticsNodes().isEmpty() }
    rule.onAllNodesWithTag(INTERIM_TAG).assertCountEquals(0)
    assertEquals(decodes + 1uL, layer.imageStats().snapshot().decodes)
  }
}
