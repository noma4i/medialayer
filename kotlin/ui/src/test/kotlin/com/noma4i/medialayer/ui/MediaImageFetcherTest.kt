package com.noma4i.medialayer.ui

import com.noma4i.medialayer.testing.TestFetcher
import android.content.Context
import android.graphics.Bitmap
import android.os.Looper
import androidx.test.core.app.ApplicationProvider
import coil3.request.CachePolicy
import coil3.request.ErrorResult
import coil3.request.ImageRequest
import coil3.request.ImageResult
import coil3.request.SuccessResult
import java.io.ByteArrayOutputStream
import java.util.concurrent.CopyOnWriteArrayList
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import com.noma4i.medialayer.*
import coil3.size.Scale
import java.nio.file.Files

internal fun pngBytes(width: Int = 64, height: Int = 64): ByteArray {
  val out = ByteArrayOutputStream()
  Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888).compress(Bitmap.CompressFormat.PNG, 100, out)
  return out.toByteArray()
}

@RunWith(MediaLayerTestRunner::class)
class MediaImageFetcherTest {
  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val requested = CopyOnWriteArrayList<Pair<String, Map<String, String>>>()
  private val stats = ImageCounters()
  private val loader = mediaImageLoader(context, ImageBytes { url, headers -> requested.add(url to headers); ImageFetchResult(pngBytes(), ImageOrigin.CACHE) }, stats = stats)

  @Test
  fun selectedHeadersSeparateDecodedImagesAndUnselectedHeadersReuseThem() {
    val media = MediaLayer.open(OpenConfig(
      dir = Files.createTempDirectory("image-identity").toString(),
      fetcher = TestFetcher(),
      settings = SharedPreferencesSettings(context, "image-identity"),
      slots = null, device = DeviceMemory(4096u, null, false),
      defaults = defaultLimits(), pool = defaultPoolSettings(), namespaceHeaders = listOf("Authorization")
    ))
    for (pixels in listOf(null, PixelSize(16u, 16u))) {
      val url = "https://o.test/$pixels.png"
      for ((account, ignored) in listOf("A" to "1", "B" to "1", "A" to "2")) {
        val source = object : MediaSource {
          override val url = url
          override val kind = Kind.IMAGE
          override val headers = mapOf("Authorization" to account, "X-Ignored" to ignored)
        }
        val request = imageRequest(context, source, pixels, Scale.FILL, media, admit = freeAdmit)
        assertTrue(load(request) is SuccessResult)
      }
      assertEquals(listOf("A", "B"), requested.filter { it.first == url }.map { it.second["Authorization"] })
    }
  }

  private fun load(request: ImageRequest, with: coil3.ImageLoader = loader): ImageResult {
    var result: ImageResult? = null
    with.enqueue(request.newBuilder().listener(onSuccess = { _, success -> result = success }, onError = { _, failure -> result = failure }).build())
    val deadline = System.nanoTime() + 10_000_000_000L
    while (result == null) {
      check(System.nanoTime() < deadline) { "the image request never finished" }
      shadowOf(Looper.getMainLooper()).idle()
      Thread.yield()
    }
    drainFfi()
    return result!!
  }

  @Test
  fun inv_image_49_aFileImageGoesThroughTheCoreAndAFileAboveTheResponseLimitFails() {
    val media = MediaLayer.open(OpenConfig(
      dir = Files.createTempDirectory("local-image").toString(),
      fetcher = TestFetcher(),
      settings = SharedPreferencesSettings(context, "local-image"),
      slots = null, device = DeviceMemory(4096u, null, false),
      defaults = defaultLimits(), pool = defaultPoolSettings()
    ))
    val fetched = CopyOnWriteArrayList<String>()
    val core = mediaImageLoader(context, ImageBytes { url, headers -> fetched.add(url); media.fetchImage(url, headers) })
    val small = Files.createTempFile("small", ".png").also { Files.write(it, pngBytes(8, 8)) }
    val huge = Files.createTempFile("huge", ".png").also { Files.write(it, ByteArray(16 * 1024 * 1024 + 1)) }
    fun request(file: java.nio.file.Path) = ImageRequest.Builder(context).data(MediaImageData(file.toUri().toString(), emptyMap(), media)).size(8, 8).build()

    val loaded = load(request(small), core)
    val refused = load(request(huge), core)

    assertTrue((loaded as? ErrorResult)?.throwable?.toString() ?: "", loaded is SuccessResult)
    assertTrue((refused as? ErrorResult)?.throwable is MediaException)
    assertEquals(listOf(small.toUri().toString(), huge.toUri().toString()), fetched.toList())
    Files.deleteIfExists(small)
    Files.deleteIfExists(huge)
    media.close()
  }

  @Test
  fun inv_image_12_imageBytesComeFromTheCoreSourceDecodeAtTheDeclaredSizeAndCoilKeepsNoDisk() {
    val data = MediaImageData("https://o.test/a.png", mapOf("Authorization" to "token"), RecordingLayer())

    val result = load(ImageRequest.Builder(context).data(data).size(16, 16).build())

    assertTrue((result as? ErrorResult)?.throwable?.toString() ?: "", result is SuccessResult)
    assertEquals(listOf("https://o.test/a.png" to mapOf("Authorization" to "token")), requested.toList())
    assertEquals(16, (result as SuccessResult).image.width)
    assertNull(loader.diskCache)
    assertEquals(CachePolicy.DISABLED, loader.defaults.diskCachePolicy)
  }

  @Test
  fun inv_image_13_aSecondRequestForTheSameUrlIsServedFromCoilMemoryWithoutTheCore() {
    val request = ImageRequest.Builder(context).data(MediaImageData("https://o.test/b.png", emptyMap(), RecordingLayer(), PixelSize(64u, 64u))).apply { extras[PLACEMENT_PIXELS] = 256L }.size(16, 16).build()

    load(request)
    assertEquals(256uL, stats.snapshot().placementPixels)
    load(request)

    assertEquals(1, requested.size)
    assertEquals(2uL, stats.snapshot().requests)
    assertEquals(1uL, stats.snapshot().decodes)
    assertEquals(1uL, stats.snapshot().diskHits)
    assertEquals(1uL, stats.snapshot().memoryHits)
    assertEquals(0uL, stats.snapshot().activeRequests)
    assertEquals(512uL, stats.snapshot().placementPixels)
  }
}
