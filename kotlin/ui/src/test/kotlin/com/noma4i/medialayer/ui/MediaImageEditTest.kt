package com.noma4i.medialayer.ui

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import com.noma4i.medialayer.CropFractions
import com.noma4i.medialayer.ImageFormat
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.PhotoEdit
import com.noma4i.medialayer.PixelSize
import com.noma4i.medialayer.imageEncodePlan
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
  private val orientationBefore = photoOrientation

  @Before
  fun upright() {
    photoOrientation = { android.media.ExifInterface.ORIENTATION_NORMAL }
  }

  @After
  fun restore() {
    photoOrientation = orientationBefore
  }

  private fun edit(maxSize: PixelSize? = null, enlarge: Boolean = false, crop: CropFractions? = null, outputSize: PixelSize? = null, mirror: Boolean = false, rotation: UInt = 0u) = PhotoEdit(maxSize, enlarge, crop, outputSize, mirror, rotation)

  private fun prepare(file: File, edit: PhotoEdit, ledger: EditLedger) = runBlocking { prepareImageAt(file.parentFile!!, file, 0.9, ledger.admit, edit) }

  @Test
  fun inv_send_21_aPhotoWithoutAnEditAsksOneRasterAndKeepsTheOldBehaviour() {
    val ledger = EditLedger()

    val prepared = prepare(pngFile(40, 20), edit(), ledger)
    prepared.file.deleteOnExit()

    assertEquals(listOf(40L * 20 * 4), ledger.asked.toList())
    assertEquals(1, ledger.closed.get())
  }

  private fun sizeOf(file: File): Pair<Int, Int> = BitmapFactory.decodeFile(file.path).let { it.width to it.height }

  @Test
  fun inv_send_23_theCropIsTakenAfterTheTurnAndPastTheEdgeItIsCut() {
    val file = pngFile(80, 40)
    photoOrientation = { android.media.ExifInterface.ORIENTATION_ROTATE_90 }

    val top = prepare(file, edit(crop = CropFractions(0.0, 0.0, 1.0, 0.5)), EditLedger()).file.also { it.deleteOnExit() }
    photoOrientation = { android.media.ExifInterface.ORIENTATION_NORMAL }
    val pastTheEdge = prepare(file, edit(crop = CropFractions(0.75, 0.0, 1.0, 1.0)), EditLedger()).file.also { it.deleteOnExit() }

    assertEquals(40 to 40, sizeOf(top))
    assertEquals(20 to 40, sizeOf(pastTheEdge))
    assertEquals(PixelSize(40u, 40u), photoPlan(80u, 40u, 6u, edit(crop = CropFractions(0.0, 0.0, 1.0, 0.5)))!!.output)
    assertEquals(null, photoPlan(80u, 40u, 1u, edit(crop = CropFractions(0.0, 0.0, 0.0, 1.0))))
    assertEquals(null, photoPlan(80u, 40u, 1u, edit(crop = CropFractions(Double.NaN, 0.0, 0.5, 1.0))))
  }

  @Test
  fun inv_send_24_theOutputSizeIsExactBeatsTheMaxSizeAndTheMaxSizeFitsTheCroppedRectangle() {
    val file = pngFile(80, 40)
    val half = CropFractions(0.0, 0.0, 0.5, 1.0)

    val exact = prepare(file, edit(crop = half, outputSize = PixelSize(30u, 30u)), EditLedger()).file.also { it.deleteOnExit() }
    val beatsMax = prepare(file, edit(maxSize = PixelSize(10u, 10u), outputSize = PixelSize(30u, 30u)), EditLedger()).file.also { it.deleteOnExit() }
    val fitted = prepare(file, edit(crop = half, maxSize = PixelSize(20u, 20u)), EditLedger()).file.also { it.deleteOnExit() }

    assertEquals(30 to 30, sizeOf(exact))
    assertEquals(30 to 30, sizeOf(beatsMax))
    assertEquals(20 to 20, sizeOf(fitted))
  }

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
  fun inv_send_37_aRotationTurnsThePhotoClockwiseAfterTheExifTurnAndTheCropFollowsIt() {
    val ledger = EditLedger()
    val file = pngFile(80, 40)
    fun size(edit: PhotoEdit): Pair<Int, Int> {
      val result = prepare(file, edit, ledger).file.also { it.deleteOnExit() }
      val bounds = android.graphics.BitmapFactory.Options().apply { inJustDecodeBounds = true }.also { android.graphics.BitmapFactory.decodeFile(result.path, it) }
      return bounds.outWidth to bounds.outHeight
    }

    assertEquals(40 to 80, size(edit(rotation = 90u)))
    assertEquals(40 to 40, size(edit(crop = CropFractions(0.0, 0.0, 1.0, 0.5), rotation = 90u)))
    assertEquals(80 to 40, size(edit(rotation = 180u)))
    assertEquals(40 to 80, size(edit(rotation = 270u)))
    photoOrientation = { android.media.ExifInterface.ORIENTATION_ROTATE_90 }
    assertEquals(80 to 40, size(edit(rotation = 90u)))
    assertThrows(MediaException.Fetch::class.java) { prepare(file, edit(rotation = 45u), ledger) }
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
  fun inv_send_35_aFiftyMegapixelPhotoIsDecodedBelowAThirdOfItsPixelsAndTheLedgerAsksTheDecodedRaster() {
    val file = grayJpegFile(8160, 6120)
    val fifty = 8160L * 6120
    val cases = listOf(
      edit(crop = CropFractions(0.0, 0.0, 0.75, 1.0), outputSize = PixelSize(1080u, 1080u)),
      edit(maxSize = PixelSize(1080u, 1920u)),
      edit(outputSize = PixelSize(1080u, 1080u))
    )

    for (case in cases) {
      val plan = photoPlan(8160u, 6120u, 1u, case)!!
      val decoded = decodeSampled(file, plan)
      assertTrue("${decoded.width}x${decoded.height}", decoded.width.toLong() * decoded.height < fifty / 3)
      assertEquals(plan.decode.width.toInt() to plan.decode.height.toInt(), decoded.width to decoded.height)
      assertTrue(plan.cost.toLong() < fifty * 4 / 3)
      decoded.recycle()
    }
  }

  @Test
  fun inv_send_41_prepareImageReportsTheDecodedRasterToTheImageCounters() {
    val file = grayJpegFile(8160, 6120)
    val changed = edit(outputSize = PixelSize(1080u, 1080u))
    val plan = photoPlan(8160u, 6120u, 1u, changed)!!
    val counters = com.noma4i.medialayer.ImageCounters()

    val prepared = runBlocking { prepareImageAt(file.parentFile!!, file, 0.9, EditLedger().admit, changed, counters) }
    prepared.file.deleteOnExit()

    val snapshot = counters.snapshot()
    assertEquals(1uL, snapshot.decodes)
    assertTrue("${snapshot.decodedPixels}", snapshot.decodedPixels < (8160uL * 6120uL) / 3uL)
    assertEquals(plan.decode.width.toULong() * plan.decode.height.toULong(), snapshot.decodedPixels)
  }

  @Test
  fun inv_send_35_aSampledPhotoKeepsTheOutputSizeAndAsksTheDecodedRaster() {
    val ledger = EditLedger()
    val file = jpegFile(800, 400)

    val prepared = prepare(file, edit(crop = CropFractions(0.0, 0.0, 0.5, 1.0), outputSize = PixelSize(50u, 50u)), ledger)
    prepared.file.deleteOnExit()
    val result = android.graphics.BitmapFactory.decodeFile(prepared.file.path)

    assertEquals(50 to 50, result.width to result.height)
    assertEquals(listOf(100L * 50 * 4 + 50L * 50 * 4), ledger.asked.toList())
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
}
