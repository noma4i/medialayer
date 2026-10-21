package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.Bitmap
import android.os.Looper
import androidx.test.core.app.ApplicationProvider
import coil.request.CachePolicy
import coil.request.ErrorResult
import coil.request.ImageRequest
import coil.request.ImageResult
import coil.request.SuccessResult
import java.io.ByteArrayOutputStream
import java.util.concurrent.CopyOnWriteArrayList
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import com.noma4i.medialayer.*
import coil.size.Scale
import java.nio.file.Files

internal fun pngBytes(width: Int = 64, height: Int = 64): ByteArray {
  val out = ByteArrayOutputStream()
  Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888).compress(Bitmap.CompressFormat.PNG, 100, out)
  return out.toByteArray()
}

@RunWith(RobolectricTestRunner::class)
class MediaImageFetcherTest {
  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val requested = CopyOnWriteArrayList<Pair<String, Map<String, String>>>()
  private val loader = mediaImageLoader(context, ImageBytes { url, headers -> requested.add(url to headers); pngBytes() })

  @Test
  fun selectedHeadersSeparateDecodedImagesAndUnselectedHeadersReuseThem() {
    val media = MediaLayer.open(OpenConfig(availableBytes = ULong.MAX_VALUE,
      dir = Files.createTempDirectory("image-identity").toString(),
      fetcher = object : com.noma4i.medialayer.Fetcher {
        override suspend fun fetch(request: FetchRequest): FetchResponse = throw MediaException.Fetch("unused origin")
      },
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
        val request = imageRequest(context, source, pixels, Scale.FILL, media.cacheKey(url, source.headers))
        assertTrue(load(request) is SuccessResult)
      }
      assertEquals(listOf("A", "B"), requested.filter { it.first == url }.map { it.second["Authorization"] })
    }
  }

  private fun load(request: ImageRequest): ImageResult {
    var result: ImageResult? = null
    loader.enqueue(request.newBuilder().listener(onSuccess = { _, success -> result = success }, onError = { _, failure -> result = failure }).build())
    val deadline = System.nanoTime() + 10_000_000_000L
    while (result == null) {
      check(System.nanoTime() < deadline) { "the image request never finished" }
      shadowOf(Looper.getMainLooper()).idle()
      Thread.yield()
    }
    return result!!
  }

  @Test
  fun imageBytesComeFromTheCoreSourceDecodeAtTheDeclaredSizeAndCoilKeepsNoDisk() {
    val data = MediaImageData("https://o.test/a.png", mapOf("Authorization" to "token"), "https://o.test/a.png")

    val result = load(ImageRequest.Builder(context).data(data).size(16, 16).build())

    assertTrue((result as? ErrorResult)?.throwable?.toString() ?: "", result is SuccessResult)
    assertEquals(listOf("https://o.test/a.png" to mapOf("Authorization" to "token")), requested.toList())
    assertEquals(16, (result as SuccessResult).drawable.intrinsicWidth)
    assertNull(loader.diskCache)
    assertEquals(CachePolicy.DISABLED, loader.defaults.diskCachePolicy)
  }

  @Test
  fun aSecondRequestForTheSameUrlIsServedFromCoilMemoryWithoutTheCore() {
    val request = ImageRequest.Builder(context).data(MediaImageData("https://o.test/b.png", emptyMap(), "https://o.test/b.png")).size(16, 16).build()

    load(request)
    load(request)

    assertEquals(1, requested.size)
  }
}
