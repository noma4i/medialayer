package com.noma4i.medialayer.ui

import android.content.Context
import android.os.Looper
import androidx.test.core.app.ApplicationProvider
import coil3.decode.Decoder
import coil3.fetch.SourceFetchResult
import coil3.request.ImageRequest
import coil3.request.ImageResult
import coil3.request.Options
import coil3.size.Scale
import com.noma4i.medialayer.ImageFetchResult
import com.noma4i.medialayer.ImageOrigin
import com.noma4i.medialayer.MediaException
import java.io.File
import java.util.Base64
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.After
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

internal val freeAdmit: DecodeAdmit = { AutoCloseable {} }

private val animatedWebp = Base64.getDecoder().decode("UklGRrQAAABXRUJQVlA4WAoAAAACAAAADwAADwAAQU5JTQYAAAAAAAAAAABBTk1GKAAAAAAAAAAAAAcAAAcAAGQAAANWUDhMDwAAAC8HwAEABxD9j/4HIqL/AQBBTk1GKAAAAAIAAAAAAAcAAAcAAGQAAAFWUDhMDwAAAC8HwAEABxD9j/4HIqL/AQBBTk1GKAAAAAQAAAAAAAcAAAcAAGQAAABWUDhMDwAAAC8HwAEABxD9j/4HIqL/AQA=")

private class DrawCounter : FrameSurface {
  var draws = 0
  override fun clear(x: Int, y: Int, width: Int, height: Int) {}
  override fun draw(still: ByteArray, x: Int, y: Int, blends: Boolean) { draws++ }
}

private fun pngHeader(width: Int, height: Int): ByteArray {
  val out = java.io.ByteArrayOutputStream()
  javax.imageio.ImageIO.write(java.awt.image.BufferedImage(width, height, java.awt.image.BufferedImage.TYPE_INT_ARGB), "png", out)
  return out.toByteArray()
}

private class Admissions {
  val asked = CopyOnWriteArrayList<Long>()
  val closed = AtomicInteger()
  val admit: DecodeAdmit = { bytes ->
    asked.add(bytes)
    AutoCloseable { closed.incrementAndGet() }
  }
}

@RunWith(MediaLayerTestRunner::class)
class DecodeAdmissionTest {
  private val context = ApplicationProvider.getApplicationContext<Context>()

  private val orientationBefore = photoOrientation

  @Before
  fun upright() {
    Media.failure = null
    photoOrientation = { android.media.ExifInterface.ORIENTATION_NORMAL }
  }

  @After
  fun restore() {
    Media.failure = null
    photoOrientation = orientationBefore
  }

  @Test
  fun inv_image_61_aLastFrameDecodeStopsBetweenFramesWhenItsCallerIsCancelled() {
    val surface = DrawCounter()
    assertThrows(CancellationException::class.java) {
      WebpAnimation.parse(animatedWebp).compose(surface) { if (surface.draws >= 1) throw CancellationException("the view is gone") }
    }
    assertEquals(1, surface.draws)
    val whole = DrawCounter()
    WebpAnimation.parse(animatedWebp).compose(whole)
    assertEquals(3, whole.draws)
  }

  @Test
  fun inv_image_62_aCanvasAboveThePixelLimitIsRefusedBeforeAnyBitmapIsMade() {
    val huge = animatedWebp.copyOf()
    huge[24] = 0x00
    huge[25] = 0x40
    huge[26] = 0x00
    huge[27] = 0x00
    huge[28] = 0x40
    huge[29] = 0x00

    assertThrows(IllegalArgumentException::class.java) { restFramePlan(huge, null, null, Scale.FIT) }
  }

  @Test
  fun inv_image_62_aLastFrameDecodeAsksTheLedgerForTheScaledCanvasAndDrawsAtThatScale() {
    val full = restFramePlan(animatedWebp, null, null, Scale.FIT)!!
    val half = restFramePlan(animatedWebp, 8, 8, Scale.FIT)!!

    assertEquals(1.0, full.multiplier, 0.0)
    assertEquals(0.5, half.multiplier, 0.0)
    assertEquals(full.bytes - 16L * 16 * 4 + 8L * 8 * 4, half.bytes)
    val bitmap = lastFrame(animatedWebp, half.multiplier)!!
    assertEquals(8, bitmap.width)
    assertEquals(8, bitmap.height)
  }

  @Test
  fun inv_image_63_aStillFileIsOneAllocationOfTheHeaderAndTheChunks() {
    val frame = WebpAnimation.parse(animatedWebp).frames.first()
    val before = DecodeCopies.bytes.get()

    val still = frame.stillFile()

    assertEquals(30 + frame.chunkLength, still.size)
    assertEquals(still.size.toLong(), DecodeCopies.bytes.get() - before)
  }

  @Test
  fun inv_image_64_theFetcherHandsCoilASourceThatIsNotACopyOfTheBytes() {
    val payload = ByteArray(256 * 1024) { 7 }
    val seen = CopyOnWriteArrayList<Long>()
    val spy = object : Decoder.Factory {
      override fun create(result: SourceFetchResult, options: Options, imageLoader: coil3.ImageLoader): Decoder? {
        seen.add(result.source.source().buffer.size)
        return null
      }
    }
    val loader = mediaImageLoader(context, ImageBytes { _, _ -> ImageFetchResult(payload, ImageOrigin.CACHE) })
    val request = ImageRequest.Builder(context).data(MediaImageData("https://o.test/a.png", emptyMap(), RecordingLayer())).decoderFactory(spy).size(16, 16).build()

    var result: ImageResult? = null
    loader.enqueue(request.newBuilder().listener(onSuccess = { _, success -> result = success }, onError = { _, failure -> result = failure }).build())
    val deadline = System.nanoTime() + 10_000_000_000L
    while (result == null) {
      check(System.nanoTime() < deadline) { "the image request never finished" }
      shadowOf(Looper.getMainLooper()).idle()
      Thread.yield()
    }

    assertEquals(1, seen.size)
    assertTrue("the source buffer held ${seen[0]} of ${payload.size} bytes", seen[0] < payload.size / 4)
  }

  @Test
  fun inv_tone_21_aBlockingFrameReadEndsWhenItsCallerIsCancelled() = runBlocking {
    val released = CountDownLatch(1)
    val aborts = AtomicInteger()
    val started = CountDownLatch(1)
    val job = CoroutineScope(Dispatchers.Default + Job()).launch {
      cancellableBlocking(abort = { aborts.incrementAndGet(); released.countDown() }) {
        started.countDown()
        released.await()
      }
    }
    try {
      assertTrue(started.await(5, TimeUnit.SECONDS))

      withTimeout(5_000) { job.cancelAndJoin() }

      assertEquals(1, aborts.get())
    } finally {
      released.countDown()
    }
  }

  @Test
  fun inv_memory_17_aRefusalOfTheLedgerReachesMediaFailureAndFailsTheRequest() = runBlocking {
    Media.failure = null
    val refused = MediaException.Overloaded("no room")

    val thrown = runCatching { admitReporting({ throw refused }, 10) }.exceptionOrNull()

    assertEquals(refused, thrown)
    assertEquals(refused, Media.failure)
    Media.failure = null
    admitReporting({ AutoCloseable {} }, 10).close()
    assertEquals(null, Media.failure)
  }

  private fun viewLoad(admit: DecodeAdmit, width: Int, height: Int): ImageResult {
    val loader = mediaImageLoader(context, ImageBytes { _, _ -> ImageFetchResult(pngBytes(64, 64), ImageOrigin.CACHE) }, null, com.noma4i.medialayer.ImageCounters(), PixelSizeMemo(), admit)
    val request = ImageRequest.Builder(context).data(MediaImageData("https://o.test/${width}x$height.png", emptyMap(), RecordingLayer())).size(width, height).build()
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
  fun inv_memory_13_aViewDecodeAsksTheLedgerForTheRequestedPixelsAndGivesThemBack() {
    val admissions = Admissions()

    val result = viewLoad(admissions.admit, 16, 16)

    assertTrue((result as? coil3.request.ErrorResult)?.throwable?.toString() ?: "", result is coil3.request.SuccessResult)
    assertEquals(listOf(16L * 16 * 4), admissions.asked.toList())
    assertEquals(1, admissions.closed.get())
  }

  @Test
  fun inv_memory_13_aRefusedAdmissionFailsTheViewRequest() {
    val result = viewLoad({ throw MediaException.Overloaded("no room") }, 16, 16)

    assertTrue(result is coil3.request.ErrorResult)
  }

  @Test
  fun inv_memory_13_aViewDecodeAboveThePixelLimitFailsBeforeAnyAdmission() {
    val admissions = Admissions()

    val result = viewLoad(admissions.admit, 16_385, 1)

    assertTrue(result is coil3.request.ErrorResult)
    assertTrue(admissions.asked.isEmpty())
  }

  @Test
  fun inv_send_18_aPhotoTakesOneRasterFromTheLedgerAndGivesItBack() = runBlocking {
    val file = File.createTempFile("photo", ".png").apply { writeBytes(pngHeader(40, 20)); deleteOnExit() }
    val cache = File.createTempFile("cache", "").let { it.delete(); it.mkdirs(); it }
    val admissions = Admissions()

    val outcome = runCatching { prepareImageAt(cache, file, 0.9, admissions.admit) }

    assertEquals(outcome.exceptionOrNull()?.stackTraceToString(), listOf(40L * 20 * 4), admissions.asked.toList())
    assertEquals(1, admissions.closed.get())
  }

  @Test
  fun inv_send_20_aRefusedAdmissionStopsThePhotoBeforeItDecodesAndLeavesNoFile() {
    val file = File.createTempFile("photo", ".png").apply { writeBytes(pngHeader(40, 20)); deleteOnExit() }
    val cache = File.createTempFile("cache", "").let { it.delete(); it.mkdirs(); it }

    assertThrows(MediaException.Overloaded::class.java) { runBlocking { prepareImageAt(cache, file, 0.9, { throw MediaException.Overloaded("no room") }) } }

    assertTrue(cache.list()!!.isEmpty())
  }

  @Test
  fun inv_send_19_aPhotoAboveThePixelLimitIsRefusedBeforeAnyAdmission() {
    val file = File.createTempFile("photo", ".png").apply { writeBytes(pngHeader(16_385, 1)); deleteOnExit() }
    val admissions = Admissions()

    assertThrows(MediaException.Fetch::class.java) { runBlocking { prepareImageAt(file.parentFile!!, file, 0.9, admissions.admit) } }

    assertTrue(admissions.asked.isEmpty())
  }
}
