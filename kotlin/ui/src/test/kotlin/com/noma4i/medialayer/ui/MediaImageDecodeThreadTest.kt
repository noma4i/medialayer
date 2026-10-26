package com.noma4i.medialayer.ui

import android.content.Context
import android.os.Looper
import androidx.test.core.app.ApplicationProvider
import coil3.decode.Decoder
import coil3.fetch.SourceFetchResult
import coil3.request.ImageResult
import coil3.request.Options
import coil3.size.Scale
import com.noma4i.medialayer.ImageFetchResult
import com.noma4i.medialayer.ImageOrigin
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSourceValue
import java.util.concurrent.CopyOnWriteArrayList
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

@RunWith(MediaLayerTestRunner::class)
class MediaImageDecodeThreadTest {
  private val context = ApplicationProvider.getApplicationContext<Context>()

  @Test
  fun inv_image_65_a_view_request_fetches_and_decodes_off_the_main_thread() {
    val main = Looper.getMainLooper().thread
    val fetchedOn = CopyOnWriteArrayList<Thread>()
    val decodedOn = CopyOnWriteArrayList<Thread>()
    val spy = object : Decoder.Factory {
      override fun create(result: SourceFetchResult, options: Options, imageLoader: coil3.ImageLoader): Decoder? {
        decodedOn.add(Thread.currentThread())
        return null
      }
    }
    val loader = mediaImageLoader(context, ImageBytes { _, _ -> fetchedOn.add(Thread.currentThread()); ImageFetchResult(ByteArray(64) { 7 }, ImageOrigin.CACHE) })
    val source = MediaSourceValue.remote("https://o.test/decode-thread.png", Kind.IMAGE)
    val request = imageRequest(context, source, null, Scale.FILL, { "decode-thread" }, admit = freeAdmit).newBuilder().decoderFactory(spy).size(16, 16).build()

    var result: ImageResult? = null
    loader.enqueue(request.newBuilder().listener(onSuccess = { _, success -> result = success }, onError = { _, failure -> result = failure }).build())
    val deadline = System.nanoTime() + 10_000_000_000L
    while (result == null) {
      check(System.nanoTime() < deadline) { "the image request never finished" }
      shadowOf(Looper.getMainLooper()).idle()
      Thread.yield()
    }

    assertEquals(1, decodedOn.size)
    assertEquals(1, fetchedOn.size)
    assertTrue("the decode ran on the main thread", decodedOn.none { it === main })
    assertTrue("the fetch ran on the main thread", fetchedOn.none { it === main })
  }
}
