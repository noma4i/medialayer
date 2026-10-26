package com.noma4i.medialayer.ui

import android.graphics.Bitmap
import android.os.Looper
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.testing.TestFetcher
import java.awt.image.BufferedImage
import java.io.ByteArrayOutputStream
import java.io.File
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicInteger
import javax.imageio.ImageIO
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

private fun exifJpeg(width: Int, height: Int, orientation: Int): ByteArray {
  val out = ByteArrayOutputStream()
  ImageIO.write(BufferedImage(width, height, BufferedImage.TYPE_INT_RGB), "jpg", out)
  val plain = out.toByteArray()
  val tiff = byteArrayOf('M'.code.toByte(), 'M'.code.toByte(), 0, 42, 0, 0, 0, 8, 0, 1, 0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, orientation.toByte(), 0, 0, 0, 0, 0, 0)
  val body = "Exif".toByteArray() + byteArrayOf(0, 0) + tiff
  val segment = byteArrayOf(0xFF.toByte(), 0xE1.toByte(), 0, (body.size + 2).toByte()) + body
  return plain.copyOfRange(0, 2) + segment + plain.copyOfRange(2, plain.size)
}

private fun widePng(): ByteArray {
  val out = ByteArrayOutputStream()
  ImageIO.write(BufferedImage(16_385, 1, BufferedImage.TYPE_INT_ARGB), "png", out)
  return out.toByteArray()
}

private class DecodeLedger {
  val asked = CopyOnWriteArrayList<Long>()
  val onMain = CopyOnWriteArrayList<Boolean>()
  val closed = AtomicInteger()
  val admit: DecodeAdmit = { bytes ->
    asked.add(bytes)
    onMain.add(Looper.myLooper() == Looper.getMainLooper())
    AutoCloseable { closed.incrementAndGet() }
  }
}

@RunWith(MediaLayerTestRunner::class)
class MediaImageDecodeTest {
  private val address = "https://img.example/avatar.jpg"

  @After
  fun clean() {
    Media.failure = null
  }

  private fun decode(bytes: ByteArray, maxSide: Int?, admit: DecodeAdmit?): Bitmap = runBlocking { decodeImage(ImageInput { bytes.inputStream() }, maxSide, admit) }

  @Test
  fun inv_image_70_anExifTurnOfSixAndEightGivesAnUprightBitmapWithSwappedSides() {
    for (tag in listOf(6, 8)) {
      val bitmap = decode(exifJpeg(40, 20, tag), null, null)
      assertEquals("tag $tag", 20 to 40, bitmap.width to bitmap.height)
    }
    val upright = decode(exifJpeg(40, 20, 1), null, null)
    assertEquals(40 to 20, upright.width to upright.height)
  }

  @Test
  fun inv_image_71_theLongSideIsCutToTheLimitAndASmallPictureIsKept() {
    val cut = decode(exifJpeg(400, 200, 6), 100, null)
    assertEquals(50 to 100, cut.width to cut.height)
    val kept = decode(exifJpeg(40, 20, 1), 500, null)
    assertEquals(40 to 20, kept.width to kept.height)
  }

  @Test
  fun inv_image_70_theDecodeRunsOffTheMainThreadAndAsksTheLedgerForTheTurnedRaster() {
    val admissions = DecodeLedger()

    decode(exifJpeg(40, 20, 6), null, admissions.admit)

    assertEquals(listOf(false), admissions.onMain.toList())
    assertEquals(listOf(40L * 20 * 4 * 2), admissions.asked.toList())
    assertEquals(1, admissions.closed.get())
  }

  @Test
  fun inv_image_72_aRefusedAdmissionFailsTheDecodeWithAMediaExceptionAndHoldsNothing() {
    val admissions = DecodeLedger()

    assertThrows(MediaException.Overloaded::class.java) { decode(exifJpeg(40, 20, 1), null) { throw MediaException.Overloaded("no room") } }
    assertThrows(MediaException.Fetch::class.java) { decode(widePng(), null, admissions.admit) }

    assertTrue(admissions.asked.isEmpty())
  }

  @Test
  fun inv_image_72_withoutAnOpenLayerThePublicDecodeIsClosed() {
    val file = File.createTempFile("photo", ".jpg").apply { writeBytes(exifJpeg(40, 20, 1)); deleteOnExit() }

    assertThrows(MediaException.Closed::class.java) { runBlocking { MediaImageDecode.decode(file, 10) } }
  }

  @Test
  fun inv_image_73_aLoadedBitmapIsCutToTheSideAndACircleIsASquare() = runBlocking {
    val admissions = DecodeLedger()
    val plain = MediaImageLoad.bitmapOf(exifJpeg(80, 40, 1), 20, false, admissions.admit)
    val round = MediaImageLoad.bitmapOf(exifJpeg(80, 40, 1), 20, true, admissions.admit)

    assertEquals(20 to 10, plain.width to plain.height)
    assertEquals(20 to 20, round.width to round.height)
    assertEquals(2, admissions.closed.get())
  }

  @Test
  fun inv_image_73_loadBitmapWorksWithoutAnOpenLayer() = runBlocking {
    val fetcher = TestFetcher().apply { serve(address, exifJpeg(80, 40, 6)) }

    val bitmap = MediaImageLoad.loadBitmap(address, java.nio.file.Files.createTempDirectory("bitmap-load").toFile(), 30, fetcher = fetcher)

    assertEquals(15 to 30, bitmap.width to bitmap.height)
  }
}
