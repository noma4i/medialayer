package com.noma4i.medialayer.ui

import android.content.Context
import android.os.Looper
import androidx.test.core.app.ApplicationProvider
import coil.request.ImageResult
import coil.request.SuccessResult
import coil.size.Scale
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSource
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

@RunWith(MediaLayerTestRunner::class)
class ImagePixelsTest {
  @Test
  fun inv_image_11_grid_decoded_pixels_fit_placement() {
    val context = ApplicationProvider.getApplicationContext<Context>()
    val bytes = pngBytes(1080, 1920)
    val stats = com.noma4i.medialayer.ImageCounters()
    val loader = mediaImageLoader(context, ImageBytes { _, _ -> com.noma4i.medialayer.ImageFetchResult(bytes, com.noma4i.medialayer.ImageOrigin.NETWORK) }, stats = { stats }).newBuilder().allowHardware(false).build()
    var decodedPixels = 0L
    val placementPixels = 64L * 352 * 352
    try {
      repeat(64) { index ->
        val source = object : MediaSource {
          override val url = "https://o.test/tile-$index.png"
          override val kind = Kind.IMAGE
          override val headers = emptyMap<String, String>()
          override val size = 1080 to 1920
        }
        val request = imageRequest(context, source, null, Scale.FILL, source.url, 352L * 352).newBuilder().size(352, 352).build()
        var result: ImageResult? = null
        loader.enqueue(request.newBuilder().listener(onSuccess = { _, value -> result = value }, onError = { _, value -> result = value }).build())
        val deadline = System.nanoTime() + 10_000_000_000L
        while (result == null) {
          check(System.nanoTime() < deadline) { "image request did not finish" }
          shadowOf(Looper.getMainLooper()).idle()
          Thread.yield()
        }
        val drawable = (result as SuccessResult).drawable
        decodedPixels += drawable.intrinsicWidth.toLong() * drawable.intrinsicHeight
      }
      org.junit.Assert.assertEquals("grid_decoded_pixels=$decodedPixels placement_pixels=$placementPixels", 64L * 352 * 626, decodedPixels)
      org.junit.Assert.assertEquals(64uL, stats.snapshot().decodes)
      org.junit.Assert.assertEquals(decodedPixels.toULong(), stats.snapshot().decodedPixels)
      org.junit.Assert.assertEquals(placementPixels.toULong(), stats.snapshot().placementPixels)
      org.junit.Assert.assertEquals(0uL, stats.snapshot().activeRequests)
    } finally {
      loader.shutdown()
    }
  }
}
