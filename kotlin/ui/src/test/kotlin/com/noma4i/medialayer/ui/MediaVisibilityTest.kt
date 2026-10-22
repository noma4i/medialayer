package com.noma4i.medialayer.ui

import coil3.request.allowHardware
import android.content.Context
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSource
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private data class Animated(override val url: String) : MediaSource {
  override val kind: Kind get() = Kind.IMAGE
  override val headers: Map<String, String> get() = emptyMap()
}

private const val FAR = 5_000

@RunWith(MediaLayerTestRunner::class)
class MediaVisibilityTest {
  @get:Rule val rule = createComposeRule()

  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val layer = RecordingLayer()
  private val slots = ExoPlayerSlotFactory(context, startMuted = true).onMainLooper()
  private val looper = shadowOf(android.os.Looper.getMainLooper())
  private val started = AtomicInteger()
  private val cancelled = AtomicInteger()
  private val gate = CompletableDeferred<Unit>()
  private val url = "https://o.test/anim.gif"
  private val page = Animated(url)

  private val loader by lazy {
    mediaImageLoader(context, ImageBytes { _, _ ->
      started.incrementAndGet()
      try { gate.await() } catch (e: CancellationException) { cancelled.incrementAndGet(); throw e }
      com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK)
    }, stats = layer.imageStats()).newBuilder().allowHardware(false).build()
  }

  private fun claims() = layer.calls.filter { it.startsWith("visible:$url:") }

  private fun settle() = repeat(20) { looper.idleFor(java.time.Duration.ofMillis(50)); Thread.sleep(25) }

  private fun counters() = "started=${started.get()} cancelled=${cancelled.get()} decodes=${layer.imageStats().snapshot().decodes} claims=${claims()}"

  @Test
  fun inv_image_09_inv_image_13_aPageThatLeavesTheViewportReleasesItsKeyCancelsItsRequestAndNeverDecodes() {
    var x by mutableIntStateOf(0)
    var mounted by androidx.compose.runtime.mutableStateOf(true)
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
        if (mounted) Box(Modifier.fillMaxSize()) { Box(Modifier.offset { IntOffset(x, 0) }) { MediaImage(page, Modifier.size(50.dp)) } }
      }
    }
    rule.waitUntil(5000) { looper.idle(); started.get() == 1 && claims() == listOf("visible:$url:true") }

    x = FAR
    rule.waitForIdle()
    settle()
    gate.complete(Unit)
    settle()

    assertEquals("started=1 cancelled=1 decodes=0 claims=[visible:$url:true, visible:$url:false]", counters())
    assertEquals(1uL, layer.imageStats().snapshot().cancellations)
    assertEquals(0uL, layer.imageStats().snapshot().activeRequests)
    org.junit.Assert.assertTrue(layer.imageStats().snapshot().activeObservers > 0uL)
    mounted = false
    rule.waitForIdle()
    assertEquals(0uL, layer.imageStats().snapshot().activeObservers)
  }

  @Test
  fun aPageThatExitedAndReturnsAsksAgain() {
    var x by mutableIntStateOf(0)
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
        Box(Modifier.fillMaxSize()) { Box(Modifier.offset { IntOffset(x, 0) }) { MediaImage(page, Modifier.size(50.dp)) } }
      }
    }
    rule.waitUntil(5000) { looper.idle(); started.get() == 1 }
    x = FAR
    rule.waitForIdle()
    settle()
    assertEquals("started=1 cancelled=1 decodes=0 claims=[visible:$url:true, visible:$url:false]", counters())

    x = 0
    rule.waitForIdle()
    settle()
    gate.complete(Unit)
    rule.waitUntil(5000) { looper.idle(); layer.calls.any { it == "itemState:$url:Ready" } }

    assertEquals("started=2 cancelled=1 decodes=1 claims=[visible:$url:true, visible:$url:false, visible:$url:true]", counters())
  }

  @Test
  fun aPageThatWasNeverVisibleStartsWorkWhenItScrollsIn() {
    var x by mutableIntStateOf(FAR)
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
        Box(Modifier.fillMaxSize()) { Box(Modifier.offset { IntOffset(x, 0) }) { MediaImage(page, Modifier.size(50.dp)) } }
      }
    }
    settle()
    assertEquals("started=0 cancelled=0 decodes=0 claims=[]", counters())

    x = 0
    rule.waitForIdle()
    settle()
    gate.complete(Unit)
    rule.waitUntil(5000) { looper.idle(); layer.calls.any { it == "itemState:$url:Ready" } }

    assertEquals("started=1 cancelled=0 decodes=1 claims=[visible:$url:true]", counters())
  }

  @Test
  fun aLeavingOwnerDoesNotCancelOrReleaseTheKeyOfAnOwnerThatStays() {
    var x by mutableIntStateOf(0)
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) {
        Row {
          Box(Modifier.offset { IntOffset(x, 0) }) { MediaImage(page, Modifier.size(50.dp)) }
          MediaImage(page, Modifier.size(50.dp))
        }
      }
    }
    rule.waitUntil(5000) { looper.idle(); started.get() == 2 }

    x = FAR
    rule.waitForIdle()
    settle()
    gate.complete(Unit)
    rule.waitUntil(5000) { looper.idle(); layer.calls.any { it == "itemState:$url:Ready" } }
    settle()

    assertEquals("started=2 cancelled=1 decodes=1 claims=[visible:$url:true]", counters())
  }
}
