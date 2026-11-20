package com.noma4i.medialayer.ui

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import com.noma4i.medialayer.CropFractions
import com.noma4i.medialayer.ImageFormat
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.PhotoEdit
import com.noma4i.medialayer.PixelSize
import com.noma4i.medialayer.imageEncodePlan
import com.noma4i.medialayer.jpegQuality
import com.noma4i.medialayer.photoPlan
import java.awt.image.BufferedImage
import java.io.ByteArrayOutputStream
import java.io.File
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicInteger
import javax.imageio.ImageIO
import kotlin.math.roundToInt
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith

private class EditLedger {
  val asked = CopyOnWriteArrayList<Long>()
  val closed = AtomicInteger()
  val admit: DecodeAdmit = { bytes ->
    asked.add(bytes)
    AutoCloseable { closed.incrementAndGet() }
  }
}

private fun pngFile(width: Int, height: Int): File {
  val out = ByteArrayOutputStream()
  ImageIO.write(BufferedImage(width, height, BufferedImage.TYPE_INT_ARGB), "png", out)
  return File.createTempFile("edit", ".png").apply { writeBytes(out.toByteArray()); deleteOnExit() }
}

private fun grayJpegFile(width: Int, height: Int): File =
  File.createTempFile("gray", ".jpg").apply { ImageIO.write(BufferedImage(width, height, BufferedImage.TYPE_BYTE_GRAY), "jpg", this); deleteOnExit() }

private fun jpegFile(width: Int, height: Int): File =
  File.createTempFile("edit", ".jpg").apply { ImageIO.write(BufferedImage(width, height, BufferedImage.TYPE_INT_RGB), "jpg", this); deleteOnExit() }

@RunWith(MediaLayerTestRunner::class)
class MediaImageEditTest {
  private fun edit(maxSize: PixelSize? = null, enlarge: Boolean = false, crop: CropFractions? = null, outputSize: PixelSize? = null, mirror: Boolean = false, rotation: UInt = 0u) = PhotoEdit(maxSize, enlarge, crop, outputSize, mirror, rotation)

  private fun sizeOf(file: File): Pair<Int, Int> = BitmapFactory.decodeFile(file.path).let { it.width to it.height }

  @Test
  fun inv_send_30_anEncodedImageIsUprightByItsExifAndAJpegFlattensTransparencyOntoWhite() = runBlocking {
    val ledger = EditLedger()
    val transparent = Bitmap.createBitmap(8, 8, Bitmap.Config.ARGB_8888).apply { setHasAlpha(true) }
    val directory = File.createTempFile("cache", "").let { it.delete(); it.mkdirs(); it }

    val turned = encodeBytes(exifJpeg(80, 40, 6), ImageFormat.PNG, 0.9, ledger.admit).let { BitmapFactory.decodeByteArray(it, 0, it.size) }
    val flat = encodeBitmapAt(directory, transparent, ImageFormat.JPEG, 0.9, ledger.admit).file.also { it.deleteOnExit() }

    assertEquals(40 to 80, turned.width to turned.height)
    assertEquals(8 to 8, BitmapFactory.decodeFile(flat.path).let { it.width to it.height })
    assertTrue(imageEncodePlan(8u, 8u, 1u, true, ImageFormat.JPEG)!!.flatten)
    assertFalse(imageEncodePlan(8u, 8u, 1u, true, ImageFormat.PNG)!!.flatten)
    assertEquals(listOf(80L * 40 * 4 * 2, 8L * 8 * 4 * 2), ledger.asked.toList())
  }

  @Test
  fun inv_send_29_aPngHasThePngSignatureAndAJpegTheJpegOne() = runBlocking {
    val ledger = EditLedger()
    val bitmap = Bitmap.createBitmap(80, 40, Bitmap.Config.ARGB_8888).apply { setHasAlpha(true) }

    val png = encodeBitmapAt(File.createTempFile("cache", "").let { it.delete(); it.mkdirs(); it }, bitmap, ImageFormat.PNG, 0.9, ledger.admit)
    val jpeg = encodeBitmapAt(png.file.parentFile!!, bitmap, ImageFormat.JPEG, 0.9, ledger.admit)
    png.file.deleteOnExit()
    jpeg.file.deleteOnExit()

    assertArrayEquals(byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A), png.file.readBytes().copyOf(8))
    assertArrayEquals(byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 0xFF.toByte()), jpeg.file.readBytes().copyOf(3))
    assertEquals(listOf(80L * 40 * 4, 80L * 40 * 4 * 2), ledger.asked.toList())
    assertEquals(2, ledger.closed.get())
  }

  @Test
  fun inv_send_32_theBytesOfAnImageAreEncodedAgainAndARefusedAdmissionStopsIt() = runBlocking {
    val ledger = EditLedger()
    val source = exifJpeg(40, 20, 1)

    val png = encodeBytes(source, ImageFormat.PNG, 0.9, ledger.admit)

    assertThrows(MediaException.Overloaded::class.java) { runBlocking { encodeBytes(source, ImageFormat.PNG, 0.9) { throw MediaException.Overloaded("no room") } } }
    assertArrayEquals(byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47), png.copyOf(4))
    assertEquals(listOf(40L * 20 * 4), ledger.asked.toList())
  }

  @Test
  fun inv_send_43_anEncodeWithoutALayerGivesPngAndJpegBytesTurnedUprightAndEqualToTheEncodeThroughTheLayerBody() = runBlocking {
    val sideways = exifJpeg(40, 20, 6)

    val png = MediaImageEncode.encode(sideways, ImageFormat.PNG)
    val jpeg = MediaImageEncode.encode(sideways, ImageFormat.JPEG, 0.8)

    assertArrayEquals(byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A), png.copyOf(8))
    assertArrayEquals(byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 0xFF.toByte()), jpeg.copyOf(3))
    val turned = BitmapFactory.decodeByteArray(png, 0, png.size)
    assertEquals(20 to 40, turned.width to turned.height)
    assertArrayEquals(encodeBytes(sideways, ImageFormat.PNG, 0.9) { AutoCloseable {} }, png)
    assertArrayEquals(encodeBytes(sideways, ImageFormat.JPEG, 0.8) { AutoCloseable {} }, jpeg)
  }

  @Test
  fun inv_send_44_anEncodeWithoutALayerRefusesARasterAboveTheLimitWithAMediaException() = runBlocking {
    val small = exifJpeg(40, 20, 1)
    val large = exifJpeg(2000, 2000, 6)

    assertThrows(MediaException.Overloaded::class.java) { runBlocking { MediaImageEncode.encode(small, limitBytes = 40L * 20 * 4 - 1) } }
    assertThrows(MediaException.Overloaded::class.java) { runBlocking { MediaImageEncode.encode(large) } }
    assertArrayEquals(byteArrayOf(0xFF.toByte(), 0xD8.toByte(), 0xFF.toByte()), MediaImageEncode.encode(small, limitBytes = 40L * 20 * 4).copyOf(3))
  }

  @Test
  fun inv_send_31_theSizeOfBytesEqualsTheSizeOfTheDecodeWithoutDecoding() = runBlocking {
    val turned = exifJpeg(40, 20, 6)
    val plain = exifJpeg(40, 20, 1)

    val decoded = decodeImage(ImageInput { turned.inputStream() }, null, null)
    val size = MediaImageDecode.imageSize(turned)!!

    assertEquals(decoded.width to decoded.height, size.width.toInt() to size.height.toInt())
    assertEquals(20u to 40u, size.width to size.height)
    assertEquals(40u to 20u, MediaImageDecode.imageSize(plain)!!.let { it.width to it.height })
  }

  private fun photo(): Bitmap = Bitmap.createBitmap(24, 16, Bitmap.Config.ARGB_8888).apply {
    eraseColor(android.graphics.Color.RED)
    for (x in 12 until 24) for (y in 0 until 16) setPixel(x, y, android.graphics.Color.BLUE)
  }

  @Test
  fun inv_send_45_theEncodedPhotoFileHoldsTheBytesOfTheCompressedBitmap() = runBlocking {
    val cache = kotlin.io.path.createTempDirectory("encoded").toFile()
    val bitmap = photo()
    val ledger = EditLedger()

    for ((format, expected) in listOf(
      ImageFormat.PNG to ByteArrayOutputStream().also { assertTrue(bitmap.compress(Bitmap.CompressFormat.PNG, 100, it)) }.toByteArray(),
      ImageFormat.JPEG to ByteArrayOutputStream().also { assertTrue(bitmap.compress(Bitmap.CompressFormat.JPEG, (jpegQuality(0.7) * 100).roundToInt(), it)) }.toByteArray()
    )) {
      val prepared = encodeBitmapAt(cache, bitmap, format, 0.7, ledger.admit)

      assertTrue(prepared.isNew)
      assertEquals(cache, prepared.file.parentFile)
      assertTrue(prepared.file.name.startsWith("encoded-"))
      assertArrayEquals("$format", expected, prepared.file.readBytes())
    }
    assertEquals(2, cache.listFiles()!!.size)
  }

  @Test
  fun inv_send_46_aFailedOrCancelledEncodeLeavesNoFile() = runBlocking {
    val cache = kotlin.io.path.createTempDirectory("encoded").toFile()
    val refused: DecodeAdmit = { throw MediaException.Fetch("refused") }
    assertThrows(MediaException::class.java) { runBlocking { encodeBitmapAt(cache, photo(), ImageFormat.JPEG, 0.9, refused) } }
    assertEquals(0, cache.listFiles()!!.size)

    val missing = File(cache, "absent")
    assertThrows(java.io.IOException::class.java) { runBlocking { encodeBitmapAt(missing, photo(), ImageFormat.PNG, 0.9, EditLedger().admit) } }
    assertFalse(missing.exists())

    val scope = kotlinx.coroutines.CoroutineScope(kotlinx.coroutines.Job())
    val cancelling: DecodeAdmit = { scope.coroutineContext[kotlinx.coroutines.Job]!!.cancel(); AutoCloseable {} }
    val running = scope.async { encodeBitmapAt(cache, photo(), ImageFormat.JPEG, 0.9, cancelling) }
    assertThrows(java.util.concurrent.CancellationException::class.java) { runBlocking { running.await() } }
    assertEquals(0, cache.listFiles()!!.size)
  }
}
