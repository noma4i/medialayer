package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.Bitmap
import android.os.Looper
import androidx.test.core.app.ApplicationProvider
import coil3.ImageLoader
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSource
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private data class ExportSource(override val url: String) : MediaSource {
  override val kind = Kind.IMAGE
  override val headers: Map<String, String> = emptyMap()
}

@RunWith(MediaLayerTestRunner::class)
class MediaImageExportTest {
  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val layer = RecordingLayer().also { it.image = pngBytes() }
  private val fetches = AtomicInteger()
  private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)

  @After
  fun stop() = scope.coroutineContext[Job]!!.cancel()

  private fun loader(failing: Boolean = false): ImageLoader = mediaImageLoader(context, ImageBytes { url, headers ->
    fetches.incrementAndGet()
    if (failing) error("origin down")
    layer.fetchImage(url, headers)
  })

  private fun <T> settle(result: CompletableDeferred<T>): T {
    val deadline = System.nanoTime() + 10_000_000_000L
    while (!result.isCompleted) {
      check(System.nanoTime() < deadline) { "the export never finished" }
      shadowOf(Looper.getMainLooper()).idle()
      Thread.yield()
    }
    return result.getCompleted()
  }

  private fun export(imageLoader: ImageLoader, url: String, cancelled: Boolean = false): Bitmap? {
    val result = CompletableDeferred<Bitmap?>()
    scope.launch(Dispatchers.Main.immediate) {
      if (cancelled) coroutineContext[Job]!!.cancel()
      result.complete(layer.image(context, ExportSource(url), 8f, 8f, 1f, imageLoader))
    }
    return settle(result)
  }

  @Test
  fun theCallReturnsTheDecodedBitmapOfALoadableSourceWithoutAView() {
    val bitmap = export(loader(), "https://o.test/a.png")

    assertNotNull(bitmap)
    assertEquals(8, bitmap!!.width)
  }

  @Test
  fun inv_image_38_the_export_never_changes_the_state_of_a_place() {
    val imageLoader = loader()

    export(imageLoader, "https://o.test/a.png")
    export(imageLoader, "https://o.test/a.png")
    export(loader(failing = true), "https://o.test/bad.png")

    assertEquals(emptyList<String>(), layer.calls.filter { it.startsWith("itemState") })
    assertEquals(emptyList<String>(), layer.mainThreadFfi.toList())
  }

  @Test
  fun theCallReturnsNullWhenTheSourceFails() {
    val bitmap = export(loader(failing = true), "https://o.test/bad.png")

    assertNull(bitmap)
  }

  @Test
  fun theCallReturnsNullAndWritesNoStateWhenTheCoroutineIsCancelled() {
    val imageLoader = loader()
    export(imageLoader, "https://o.test/c.png")
    layer.calls.clear()
    fetches.set(0)

    val warm = export(imageLoader, "https://o.test/c.png", cancelled = true)
    val cold = export(imageLoader, "https://o.test/other.png", cancelled = true)

    assertNull(warm)
    assertNull(cold)
    assertTrue(layer.calls.toString(), layer.calls.isEmpty())
    assertEquals(0, fetches.get())
  }

  @Test
  fun aRepeatedCallIsServedFromTheCacheWithNoSecondFetch() {
    val imageLoader = loader()

    val first = export(imageLoader, "https://o.test/d.png")
    val second = export(imageLoader, "https://o.test/d.png")

    assertNotNull(first)
    assertNotNull(second)
    assertEquals(1, fetches.get())
  }

  @Test
  fun aFileSourceIsFetchedFromTheCoreByItsFileUrlWithoutTheProxy() {
    val bitmap = export(loader(), "file:///data/local/tmp/a.png")

    assertNotNull(bitmap)
    assertEquals(listOf("fetchImage:file:///data/local/tmp/a.png"), layer.calls.filter { it.startsWith("fetchImage") })
  }
}
