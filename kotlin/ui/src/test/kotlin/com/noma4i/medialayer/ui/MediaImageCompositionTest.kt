package com.noma4i.medialayer.ui

import android.content.Context
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.runtime.Composer
import androidx.compose.runtime.currentComposer
import androidx.compose.runtime.tooling.CompositionData
import androidx.compose.runtime.tooling.CompositionGroup
import androidx.compose.runtime.CompositionTracer
import androidx.compose.runtime.InternalComposeTracingApi
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSource
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private data class SizedImage(override val url: String, override val size: Pair<Int, Int>? = 24 to 24) : MediaSource {
  override val kind = Kind.IMAGE
  override val headers = emptyMap<String, String>()
}

@RunWith(MediaLayerTestRunner::class)
class MediaImageCompositionTest {
  @get:Rule val rule = createComposeRule()

  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val layer = RecordingLayer()
  private val pending = CompletableDeferred<Unit>()
  private val loader by lazy { mediaImageLoader(context, ImageBytes { _, _ -> pending.await(); com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK) }) }
  private val looper = shadowOf(android.os.Looper.getMainLooper())

  private fun waitUntil(timeoutMillis: Long, condition: () -> Boolean) {
    try {
      rule.waitUntil(timeoutMillis, condition)
    } catch (error: androidx.compose.ui.test.ComposeTimeoutException) {
      reportComposeTimeout(error, "cacheKeyReads=${layer.cacheKeyReads.get()} calls=${layer.calls} pending=${pending.isCompleted}")
    }
  }

  @Test
  fun inv_image_35_returningImageDoesNotRepeatReady() {
    val offset = mutableStateOf(0)
    val url = "https://o.test/return.png"
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader) {
        MediaImage(SizedImage(url), Modifier.offset(y = offset.value.dp).size(50.dp))
      }
    }
    pending.complete(Unit)
    waitUntil(COMPOSE_WAIT_MS) { looper.idle(); layer.calls.contains("itemState:$url:Ready") }
    rule.runOnIdle { offset.value = 10000 }
    waitUntil(COMPOSE_WAIT_MS) { looper.idle(); layer.calls.contains("visible:$url:false") }
    rule.runOnIdle { offset.value = 0 }
    waitUntil(COMPOSE_WAIT_MS) { looper.idle(); layer.calls.count { it == "visible:$url:true" } == 2 }
    rule.waitForIdle()
    assertEquals(1, layer.calls.count { it == "itemState:$url:Ready" })
  }

  @Test
  @OptIn(InternalComposeTracingApi::class)
  fun inv_image_08_sizedImagesComposeOnceInALazyList() {
    val counts = IntArray(2)
    Composer.setTracer(object : CompositionTracer {
      override fun isTraceInProgress() = true
      override fun traceEventStart(key: Int, dirty1: Int, dirty2: Int, info: String) {
        if (info.startsWith("com.noma4i.medialayer.ui.MediaImage (")) counts[0]++
        if (info.startsWith("com.noma4i.medialayer.ui.ImageBody (")) counts[1]++
      }
      override fun traceEventEnd() {}
    })
    try {
      rule.setContent {
        MediaLayerProvider(CompletableDeferred(layer), loader) {
          LazyColumn {
            items(1) { index ->
              MediaImage(SizedImage("https://o.test/$index.png"), Modifier.size(50.dp))
            }
          }
        }
      }
      waitUntil(COMPOSE_WAIT_MS) { looper.idle(); layer.calls.count { it.startsWith("visible:") && it.endsWith(":true") } == 1 }
      rule.waitForIdle()

      assertEquals(listOf(1, 1), counts.toList())
    } finally {
      Composer.setTracer(null)
    }
  }

  @Test
  fun inv_image_39_composingImagesCallsNoStatsAndAtMostOneCacheKeyPerImageOnTheMainThread() {
    val count = mutableStateOf(1)
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader) {
        Column { for (index in 0 until count.value) MediaImage(SizedImage("https://o.test/ffi-$index.png"), Modifier.size(10.dp)) }
      }
    }
    waitUntil(COMPOSE_WAIT_MS) { looper.idle(); layer.calls.contains("visible:https://o.test/ffi-0.png:true") }
    layer.mainThreadFfi.clear()
    rule.runOnIdle { count.value = 20 }
    waitUntil(COMPOSE_WAIT_MS) { looper.idle(); layer.calls.count { it.startsWith("visible:") && it.endsWith(":true") } == 20 }
    rule.waitForIdle()
    assertEquals(0, layer.mainThreadFfi.count { it == "imageStats" })
    assertEquals(0, layer.mainThreadFfi.count { it == "cacheKey" })
  }

  @Test
  fun inv_image_41_tilesComposeWithoutRememberingAnImagePainter() {
    val tiles = 12
    lateinit var data: CompositionData
    rule.setContent {
      data = currentComposer.compositionData
      MediaLayerProvider(CompletableDeferred(layer), loader) {
        Column { for (index in 0 until tiles) MediaImage(SizedImage("https://o.test/tile-$index.png"), Modifier.size(10.dp)) }
      }
    }
    waitUntil(COMPOSE_WAIT_MS) { looper.idle(); layer.calls.count { it.startsWith("visible:") && it.endsWith(":true") } == tiles }
    rule.waitForIdle()

    fun painters(group: CompositionGroup): Int =
      group.data.count { it?.javaClass?.name?.endsWith(".AsyncImagePainter") == true } + group.compositionGroups.sumOf { painters(it) }
    assertEquals(0, data.compositionGroups.sumOf { painters(it) })
  }

  @Test
  fun inv_image_40_imagesCallFfiOnlyForTheCacheKeyOnTheMainThreadAndKeepClaimOrder() {
    val total = 10
    val urls = (0 until total).map { "https://o.test/inv40-$it.png" }
    val shown = mutableStateOf(0)
    val generation = mutableStateOf(0)
    val pixelMainCalls = java.util.concurrent.atomic.AtomicInteger()
    val pixelCalls = java.util.concurrent.atomic.AtomicInteger()
    val sizes = PixelSizeMemo { width, height, scale, declared, crop ->
      pixelCalls.incrementAndGet()
      if (android.os.Looper.myLooper() == android.os.Looper.getMainLooper()) pixelMainCalls.incrementAndGet()
      com.noma4i.medialayer.imagePixelSize(width, height, scale, declared, crop)
    }
    val counting = mediaImageLoader(context, ImageBytes { _, _ -> com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK) }, null, layer.imageStats(), sizes)
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), counting) {
        Column { for (index in 0 until shown.value) MediaImage(SizedImage(if (generation.value == 0) urls[index] else "${urls[index]}?changed", if (index % 2 == 0) 24 to 24 else null), Modifier.size(10.dp)) }
      }
    }
    layer.mainThreadFfi.clear()
    fun count(call: String) = layer.calls.count { it == call }
    fun settled(visible: Boolean, round: Int) = urls.all { count("visible:$it:$visible") == round }
    for (round in 1..2) {
      rule.runOnIdle { shown.value = total }
      waitUntil(COMPOSE_WAIT_MS) { looper.idle(); settled(true, round) && urls.all { count("itemState:$it:Ready") == round } }
      rule.runOnIdle { shown.value = 0 }
      waitUntil(COMPOSE_WAIT_MS) { looper.idle(); settled(false, round) }
    }
    waitUntil(COMPOSE_WAIT_MS) { looper.idle(); drainFfi(); layer.counterWork.isNotEmpty() && layer.counterObserved == 0L }

    rule.runOnIdle { shown.value = total }
    waitUntil(COMPOSE_WAIT_MS) { looper.idle(); urls.all { count("itemState:$it:Ready") == 3 } }
    rule.runOnIdle { generation.value = 1 }
    waitUntil(COMPOSE_WAIT_MS) { looper.idle(); urls.all { count("itemState:$it?changed:Ready") == 1 } }
    assertEquals(emptyList<String>(), layer.mainThreadFfi.toList())
    assertEquals(true, pixelCalls.get() > 0)
    assertEquals(0, pixelMainCalls.get())
    for (url in urls) assertEquals(listOf("visible:$url:true", "visible:$url:false", "visible:$url:true", "visible:$url:false", "visible:$url:true", "visible:$url:false"), layer.calls.filter { it.startsWith("visible:$url:") })
  }

  @Test
  fun inv_image_39_aReturningImageIsServedFromCoilMemoryWithoutLoadingOnTheFirstFrame() {
    val shown = mutableStateOf(true)
    val url = "https://o.test/memory-hit.png"
    val fetches = java.util.concurrent.atomic.AtomicInteger()
    val sizes = PixelSizeMemo()
    val counting = mediaImageLoader(context, ImageBytes { _, _ -> fetches.incrementAndGet(); com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK) }, null, layer.imageStats(), sizes)
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), counting) {
        if (shown.value) MediaImage(SizedImage(url), Modifier.size(50.dp))
      }
    }
    waitUntil(COMPOSE_WAIT_MS) { looper.idle(); layer.calls.contains("itemState:$url:Ready") }
    rule.runOnIdle { shown.value = false }
    rule.waitForIdle()
    kotlinx.coroutines.runBlocking {
      for (index in 0 until 70) sizes.prepare(PixelSizeMemo.Input(PixelSizeMemo.Source("https://o.test/metadata-$index.png", emptyMap(), layer), null, null)) { "metadata-$index" }
    }
    rule.runOnIdle { shown.value = true }
    rule.waitForIdle()
    drainFfi()
    assertEquals(2, layer.calls.count { it == "itemState:$url:Ready" })
    assertEquals(1, fetches.get())
    assertEquals(1, layer.cacheKeyReads.get())
  }

  @Test
  fun inv_image_07_aSizedImageDoesNotCreateASubcomposeLayout() {
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader) {
        Column { MediaImage(SizedImage("https://o.test/sized.png"), Modifier.size(50.dp)) }
      }
    }
    rule.waitForIdle()
    val root = rule.onRoot().fetchSemanticsNode().layoutInfo
    fun count(node: Any): Int {
      val state = node.javaClass.declaredFields.first { it.name == "subcompositionsState" }.apply { isAccessible = true }
      val children = node.javaClass.methods.first { it.name.startsWith("getChildren$") && it.parameterCount == 0 }.invoke(node) as List<*>
      return (if (state.get(node) == null) 0 else 1) + children.filterNotNull().sumOf { count(it) }
    }

    assertEquals(0, count(root))
  }
}
