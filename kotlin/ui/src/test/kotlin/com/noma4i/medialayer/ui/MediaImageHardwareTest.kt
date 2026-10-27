package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.Bitmap
import android.graphics.drawable.ColorDrawable
import coil3.asImage
import coil3.request.bitmapConfig
import androidx.test.core.app.ApplicationProvider
import coil3.decode.DecodeResult
import coil3.decode.Decoder
import coil3.request.SuccessResult
import coil3.size.Scale
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.PixelSize
import java.util.concurrent.atomic.AtomicReference
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(MediaLayerTestRunner::class)
class MediaImageHardwareTest {
  @Test
  fun inv_image_06_list_requests_select_hardware_at_the_decoder() {
    val context = ApplicationProvider.getApplicationContext<Context>()
    var config: Bitmap.Config? = null
    val loader = mediaImageLoader(context, ImageBytes { _, _ -> com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK) })
    val source = object : MediaSource {
      override val url = "https://o.test/list.png"
      override val kind = Kind.IMAGE
      override val headers = emptyMap<String, String>()
    }
    try {
      val request = imageRequest(context, source, PixelSize(64u, 64u), Scale.FILL, RecordingLayer(), admit = freeAdmit)
        .newBuilder().decoderFactory(Decoder.Factory { _, options, _ ->
          Decoder {
            config = options.bitmapConfig
            DecodeResult(ColorDrawable().asImage(), false)
          }
        }).build()
      val completed = AtomicReference<coil3.request.ImageResult>()
      loader.enqueue(request.newBuilder().listener(
        onSuccess = { _, result -> completed.set(result) },
        onError = { _, result -> completed.set(result) }
      ).build())
      while (completed.get() == null) {
        org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle()
        Thread.yield()
      }
      val result = completed.get()
      assertTrue((result as? coil3.request.ErrorResult)?.throwable.toString(), result is SuccessResult)
      println("decoder_bitmap_config=$config")
      assertEquals(Bitmap.Config.HARDWARE, config)
    } finally {
      loader.shutdown()
    }
  }
}
