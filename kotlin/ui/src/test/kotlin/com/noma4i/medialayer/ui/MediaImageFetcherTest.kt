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
    val data = MediaImageData("https://o.test/a.png", mapOf("Authorization" to "token"))

    val result = load(ImageRequest.Builder(context).data(data).size(16, 16).build())

    assertTrue((result as? ErrorResult)?.throwable?.toString() ?: "", result is SuccessResult)
    assertEquals(listOf("https://o.test/a.png" to mapOf("Authorization" to "token")), requested.toList())
    assertEquals(16, (result as SuccessResult).drawable.intrinsicWidth)
    assertNull(loader.diskCache)
    assertEquals(CachePolicy.DISABLED, loader.defaults.diskCachePolicy)
  }

  @Test
  fun aSecondRequestForTheSameUrlIsServedFromCoilMemoryWithoutTheCore() {
    val request = ImageRequest.Builder(context).data(MediaImageData("https://o.test/b.png", emptyMap())).size(16, 16).build()

    load(request)
    load(request)

    assertEquals(1, requested.size)
  }
}
