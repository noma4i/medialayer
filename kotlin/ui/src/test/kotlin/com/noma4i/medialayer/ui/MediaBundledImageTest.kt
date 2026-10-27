package com.noma4i.medialayer.ui

import android.content.Context
import android.content.ContextWrapper
import android.content.res.Resources
import android.os.Looper
import androidx.test.core.app.ApplicationProvider
import coil3.request.ErrorResult
import coil3.request.ImageResult
import coil3.request.SuccessResult
import coil3.size.Scale
import com.noma4i.medialayer.ImageCounters
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaSourceValue
import com.noma4i.medialayer.PixelSize
import com.noma4i.medialayer.MediaState
import java.io.InputStream
import java.util.concurrent.CopyOnWriteArrayList
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private class OpenSpy(base: Context, val openedOn: CopyOnWriteArrayList<Thread>) : ContextWrapper(base) {
  @Suppress("DEPRECATION")
  private val spied = object : Resources(base.assets, base.resources.displayMetrics, base.resources.configuration) {
    override fun openRawResource(id: Int): InputStream {
      openedOn.add(Thread.currentThread())
      return base.resources.openRawResource(id)
    }
  }

  override fun getResources(): Resources = spied
}

@RunWith(MediaLayerTestRunner::class)
class MediaBundledImageTest {
  private val application = ApplicationProvider.getApplicationContext<Context>()
  private val openedOn = CopyOnWriteArrayList<Thread>()
  private val admittedOn = CopyOnWriteArrayList<Thread>()
  private val asked = CopyOnWriteArrayList<Long>()
  private val stats = ImageCounters()
  private val admit: DecodeAdmit = { bytes -> admittedOn.add(Thread.currentThread()); asked.add(bytes); AutoCloseable {} }
  private val context = OpenSpy(application, openedOn)

  private fun loader(admit: DecodeAdmit = this.admit) =
    mediaImageLoader(context, ImageBytes { url, _ -> error("the core must not be asked for the bundled $url") }, null, stats, PixelSizeMemo(), admit)

  private fun request(source: MediaSource) = imageRequest(context, source, PixelSize(16u, 16u), Scale.FILL, { source.url }, admit = admit)

  private fun load(request: coil3.request.ImageRequest, with: coil3.ImageLoader = loader()): ImageResult {
    var result: ImageResult? = null
    with.enqueue(request.newBuilder().listener(onSuccess = { _, success -> result = success }, onError = { _, failure -> result = failure }).build())
    val deadline = System.nanoTime() + 10_000_000_000L
    while (result == null) {
      check(System.nanoTime() < deadline) { "the image request never finished" }
      shadowOf(Looper.getMainLooper()).idle()
      Thread.yield()
    }
    drainFfi()
    return result as ImageResult
  }

  private val drawable get() = MediaSourceValue.raw(application, R.drawable.medialayer_test_image, Kind.IMAGE)

  @Test
  fun inv_image_74_a_drawable_is_opened_and_decoded_off_the_main_thread_at_the_view_size() {
    val main = Looper.getMainLooper().thread

    val result = load(request(drawable))

    assertTrue((result as? ErrorResult)?.throwable?.toString() ?: "", result is SuccessResult)
    assertEquals(16, (result as SuccessResult).image.width)
    assertEquals(16, result.image.height)
    assertEquals(listOf(16L * 16 * 4), asked.toList())
    assertEquals(1, openedOn.size)
    assertTrue("the resource was opened on the main thread", openedOn.none { it === main })
    assertTrue("the decode ran on the main thread", admittedOn.none { it === main })
  }

  @Test
  fun inv_image_74_an_asset_image_is_decoded_the_same_way() {
    val result = load(request(MediaSourceValue.asset("medialayer_test_image.png", Kind.IMAGE)))

    assertTrue((result as? ErrorResult)?.throwable?.toString() ?: "", result is SuccessResult)
    assertEquals(16, (result as SuccessResult).image.width)
  }

  @Test
  fun inv_image_75_a_second_show_of_the_drawable_comes_from_memory_without_a_decode() {
    val shared = loader()
    val request = request(drawable)

    load(request, shared)
    load(request, shared)

    assertEquals(1, openedOn.size)
    assertEquals(1uL, stats.snapshot().decodes)
    assertEquals(1uL, stats.snapshot().memoryHits)
  }

  @Test
  fun inv_image_75_a_refused_admission_fails_a_drawable_and_a_missing_resource_fails_too() {
    val refused = load(request(drawable), loader({ throw MediaException.Overloaded("no room") }))
    val missing = load(request(MediaSourceValue("android.resource://${application.packageName}/2130000001", Kind.IMAGE)))

    assertTrue(imageFailure((refused as ErrorResult).throwable) is MediaState.Failed)
    assertTrue(imageFailure((missing as ErrorResult).throwable) is MediaState.Failed)
  }
}
