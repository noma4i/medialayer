package com.noma4i.medialayer.ui

import android.content.Context
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSource
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private data class Thumb(override val url: String, override val blurHash: String? = null) : MediaSource {
  override val kind: Kind get() = Kind.IMAGE
  override val headers: Map<String, String> get() = emptyMap()
  override val size: Pair<Int, Int> get() = 24 to 24
}

private const val HASH = "LEHV6nWB2yk8pyo0adR*.7kCMdnj"

@RunWith(MediaLayerTestRunner::class)
class MediaTileTest {
  @get:Rule val rule = createComposeRule()

  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val layer = RecordingLayer()
  private val loader by lazy { mediaImageLoader(context, ImageBytes { _, _ -> com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK) }).newBuilder().allowHardware(false).build() }
  private val slots = ExoPlayerSlotFactory(context, startMuted = true).onMainLooper()
  private val looper = shadowOf(android.os.Looper.getMainLooper())

  private fun ready(url: String) = layer.calls.any { it == "itemState:$url:Ready" }

  private fun visibles(url: String) = layer.calls.filter { it == "visible:$url:true" || it == "visible:$url:false" }

  @Test
  fun sizedImagesPrepareNoToneWithoutDemand() {
    var full by mutableStateOf(false)
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
        Column {
          MediaImage(Thumb("https://o.test/tile.png"), Modifier.size(50.dp))
          if (full) MediaImage(Thumb("https://o.test/full.png"), Modifier.size(50.dp))
        }
      }
    }
    rule.waitUntil(5000) { looper.idle(); ready("https://o.test/tile.png") }
    full = true

    rule.waitUntil(5000) { looper.idle(); ready("https://o.test/full.png") }

    assertEquals(0, layer.toneChecks.get())
  }

  @Test
  fun aTileShowsItsBlurHashUntilReadyAndNeverAsksForAPlaceholderAfterReady() {
    val gate = CompletableDeferred<Unit>()
    val gated = mediaImageLoader(context, ImageBytes { _, _ -> gate.await(); com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK) })
    val url = "https://o.test/blur.png"
    rule.setContent { MediaLayerProvider(CompletableDeferred(layer), gated, slots.players) { MediaImage(Thumb(url, HASH), Modifier.size(50.dp)) } }

    rule.waitUntil(5000) { looper.idle(); rule.onAllNodesWithContentDescription("blurhash").fetchSemanticsNodes().isNotEmpty() }
    gate.complete(Unit)
    rule.waitUntil(5000) { looper.idle(); rule.onAllNodesWithContentDescription("blurhash").fetchSemanticsNodes().isEmpty() }
    rule.waitForIdle()

    assertTrue(ready(url))
    assertEquals(listOf(url to false), layer.placeholders.toList())
  }

  @Test
  fun anImageFollowsNoCoreStateBecauseItShowsItsOwn() {
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
        Column {
          MediaImage(Thumb("https://o.test/a.png"), Modifier.size(50.dp))
          MediaImage(Thumb("https://o.test/b.png"), Modifier.size(50.dp))
        }
      }
    }
    rule.waitUntil(5000) { looper.idle(); ready("https://o.test/a.png") && ready("https://o.test/b.png") }

    assertEquals(emptyList<String>(), layer.listeners.map { it.first })
  }

  @Test
  fun tilesOfOneUrlClaimOnceEachAndTheKeyLeavesWithTheLastTile() {
    val url = "https://o.test/shared.png"
    var count by mutableIntStateOf(3)
    var unrelated by mutableIntStateOf(0)
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
        Column { repeat(count) { MediaImage(Thumb(url), Modifier.size((50 + unrelated).dp)) } }
      }
    }
    rule.waitUntil(5000) { looper.idle(); ready(url) }

    unrelated++
    rule.waitForIdle()
    assertEquals(listOf("visible:$url:true"), visibles(url))

    count = 1
    rule.waitForIdle()
    assertEquals(listOf("visible:$url:true"), visibles(url))

    count = 0
    rule.waitForIdle()
    assertEquals(listOf("visible:$url:true", "visible:$url:false"), visibles(url))
  }
}
