package com.noma4i.medialayer.ui

import android.content.Context
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.runtime.Composer
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

private data class SizedImage(override val url: String) : MediaSource {
  override val kind = Kind.IMAGE
  override val headers = emptyMap<String, String>()
  override val size = 24 to 24
}

@RunWith(MediaLayerTestRunner::class)
class MediaImageCompositionTest {
  @get:Rule val rule = createComposeRule()

  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val layer = RecordingLayer()
  private val pending = CompletableDeferred<Unit>()
  private val loader by lazy { mediaImageLoader(context, ImageBytes { _, _ -> pending.await(); com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK) }) }
  private val looper = shadowOf(android.os.Looper.getMainLooper())

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
    rule.waitUntil(5000) { looper.idle(); layer.calls.contains("itemState:$url:Ready") }
    rule.runOnIdle { offset.value = 10000 }
    rule.waitUntil(5000) { looper.idle(); layer.calls.contains("visible:$url:false") }
    rule.runOnIdle { offset.value = 0 }
    rule.waitUntil(5000) { looper.idle(); layer.calls.count { it == "visible:$url:true" } == 2 }
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
      rule.waitUntil(5000) { looper.idle(); layer.calls.count { it.startsWith("visible:") && it.endsWith(":true") } == 1 }
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
    rule.waitUntil(5000) { looper.idle(); layer.calls.contains("visible:https://o.test/ffi-0.png:true") }
    layer.mainThreadFfi.clear()
    rule.runOnIdle { count.value = 20 }
    rule.waitUntil(5000) { looper.idle(); layer.calls.count { it.startsWith("visible:") && it.endsWith(":true") } == 20 }
    rule.waitForIdle()
    assertEquals(0, layer.mainThreadFfi.count { it == "imageStats" })
    assertEquals(true, layer.mainThreadFfi.count { it == "cacheKey" } <= 19)
  }

  @Test
  fun inv_image_40_imagesCallFfiOnlyForTheCacheKeyOnTheMainThreadAndKeepClaimOrder() {
    val total = 10
    val urls = (0 until total).map { "https://o.test/inv40-$it.png" }
    val shown = mutableStateOf(0)
    val counting = mediaImageLoader(context, ImageBytes { _, _ -> com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK) }, stats = layer.imageStats())
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), counting) {
        Column { for (index in 0 until shown.value) MediaImage(SizedImage(urls[index]), Modifier.size(10.dp)) }
      }
    }
    layer.mainThreadFfi.clear()
    fun count(call: String) = layer.calls.count { it == call }
    fun settled(visible: Boolean, round: Int) = urls.all { count("visible:$it:$visible") == round }
    for (round in 1..2) {
      rule.runOnIdle { shown.value = total }
      rule.waitUntil(10000) { looper.idle(); settled(true, round) && urls.all { count("itemState:$it:Ready") == round } }
      rule.runOnIdle { shown.value = 0 }
      rule.waitUntil(10000) { looper.idle(); settled(false, round) }
    }
    rule.waitUntil(10000) { looper.idle(); layer.counterWork.isNotEmpty() && layer.counterWork.count { it == "Observe" } == layer.counterWork.count { it == "Unobserve" } }

    assertEquals(emptyList<String>(), layer.mainThreadFfi.filter { it != "cacheKey" })
    assertEquals(true, layer.mainThreadFfi.size <= total * 2)
    for (url in urls) assertEquals(listOf("visible:$url:true", "visible:$url:false", "visible:$url:true", "visible:$url:false"), layer.calls.filter { it.startsWith("visible:$url:") })
    var open = 0
    for (work in layer.counterWork) {
      open += if (work == "Observe") 1 else -1
      assertEquals(true, open >= 0)
    }
  }

  @Test
  fun inv_image_39_aReturningImageIsServedFromCoilMemoryWithoutLoadingOnTheFirstFrame() {
    val shown = mutableStateOf(true)
    val url = "https://o.test/memory-hit.png"
    val fetches = java.util.concurrent.atomic.AtomicInteger()
    val counting = mediaImageLoader(context, ImageBytes { _, _ -> fetches.incrementAndGet(); com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK) })
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), counting) {
        if (shown.value) MediaImage(SizedImage(url), Modifier.size(50.dp))
      }
    }
    rule.waitUntil(5000) { looper.idle(); layer.calls.contains("itemState:$url:Ready") }
    rule.runOnIdle { shown.value = false }
    rule.waitForIdle()
    rule.runOnIdle { shown.value = true }
    rule.waitForIdle()
    assertEquals(2, layer.calls.count { it == "itemState:$url:Ready" })
    assertEquals(1, fetches.get())
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
