package com.noma4i.medialayer.ui

import android.graphics.Bitmap
import com.noma4i.medialayer.CropFractions
import com.noma4i.medialayer.ImageFormat
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.PhotoEdit
import com.noma4i.medialayer.PixelSize
import com.noma4i.medialayer.photoPlan
import java.awt.image.BufferedImage
import java.io.ByteArrayOutputStream
import java.io.File
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicInteger
import javax.imageio.ImageIO
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
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

@RunWith(MediaLayerTestRunner::class)
class MediaImageEditTest {
  private val orientationBefore = photoOrientation

  @Before
  fun upright() {
    photoOrientation = { android.media.ExifInterface.ORIENTATION_NORMAL }
  }

  @After
  fun restore() {
    photoOrientation = orientationBefore
  }

  private fun edit(maxSize: PixelSize? = null, enlarge: Boolean = false, crop: CropFractions? = null, outputSize: PixelSize? = null, mirror: Boolean = false) = PhotoEdit(maxSize, enlarge, crop, outputSize, mirror)

  private fun prepare(file: File, edit: PhotoEdit, ledger: EditLedger) = runBlocking { prepareImageAt(file.parentFile!!, file, 0.9, ledger.admit, edit) }

  @Test
  fun inv_send_21_aPhotoWithoutAnEditAsksOneRasterAndKeepsTheOldBehaviour() {
    val ledger = EditLedger()

    val prepared = prepare(pngFile(40, 20), edit(), ledger)
    prepared.file.deleteOnExit()

    assertEquals(listOf(40L * 20 * 4), ledger.asked.toList())
    assertEquals(1, ledger.closed.get())
  }

  @Test
  fun inv_send_25_aMirroredCropAsksTheSourceRasterAndTheOutputRaster() {
    val ledger = EditLedger()

    prepare(pngFile(80, 40), edit(crop = CropFractions(0.5, 0.0, 0.5, 1.0), mirror = true), ledger).file.deleteOnExit()

    assertEquals(listOf(80L * 40 * 4 + 40L * 40 * 4), ledger.asked.toList())
    assertEquals(1, ledger.closed.get())
  }

  @Test
  fun inv_send_33_enlargeFitsASmallPhotoIntoTheRectangleAndAsksTheEnlargedRaster() {
    val ledger = EditLedger()
    val limit = PixelSize(1080u, 1920u)

    assertEquals(PixelSize(1080u, 1920u), photoPlan(540u, 960u, 1u, edit(maxSize = limit, enlarge = true))!!.output)
    assertEquals(PixelSize(540u, 960u), photoPlan(540u, 960u, 1u, edit(maxSize = limit))!!.output)
    prepare(pngFile(40, 20), edit(maxSize = PixelSize(80u, 80u), enlarge = true), ledger).file.deleteOnExit()

    assertEquals(listOf(40L * 20 * 4 + 80L * 40 * 4), ledger.asked.toList())
  }

  @Test
  fun inv_send_26_anEditWithoutAPixelIsRefusedBeforeAnyAdmission() {
    val ledger = EditLedger()
    val file = pngFile(80, 40)

    assertThrows(MediaException.Fetch::class.java) { prepare(file, edit(crop = CropFractions(1.5, 0.0, 0.5, 1.0)), ledger) }
    assertThrows(MediaException.Fetch::class.java) { prepare(file, edit(outputSize = PixelSize(0u, 10u)), ledger) }

    assertTrue(ledger.asked.isEmpty())
  }

  @Test
  fun inv_send_22_theLastDrawGivesABitmapOfTheOutputSizeOfThePlan() {
    val source = Bitmap.createBitmap(80, 40, Bitmap.Config.ARGB_8888)
    val cases = listOf(
      edit(maxSize = PixelSize(40u, 40u)) to (40 to 20),
      edit(crop = CropFractions(0.5, 0.0, 0.5, 1.0)) to (40 to 40),
      edit(crop = CropFractions(0.0, 0.0, 0.5, 1.0), outputSize = PixelSize(30u, 30u)) to (30 to 30),
      edit(mirror = true) to (80 to 40)
    )

    for ((case, size) in cases) {
      val plan = photoPlan(80u, 40u, 1u, case)!!
      val drawn = redraw(source, plan)
      assertEquals(size, drawn.width to drawn.height)
    }
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
  fun inv_send_31_theSizeOfBytesEqualsTheSizeOfTheDecodeWithoutDecoding() = runBlocking {
    val turned = exifJpeg(40, 20, 6)
    val plain = exifJpeg(40, 20, 1)

    val decoded = decodeImage(ImageInput { turned.inputStream() }, null, null)
    val size = MediaImageDecode.imageSize(turned)!!

    assertEquals(decoded.width to decoded.height, size.width.toInt() to size.height.toInt())
    assertEquals(20u to 40u, size.width to size.height)
    assertEquals(40u to 20u, MediaImageDecode.imageSize(plain)!!.let { it.width to it.height })
  }
}
