package com.noma4i.medialayer.ui

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.PorterDuff
import android.graphics.PorterDuffXfermode
import android.media.ExifInterface
import android.net.Uri
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.ImageSize
import com.noma4i.medialayer.imageDecodePlan
import com.noma4i.medialayer.photoTurn
import java.io.ByteArrayInputStream
import java.io.File
import java.io.InputStream
import kotlin.math.max
import kotlin.math.roundToInt
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext

/**
 * A local picture as an upright [Bitmap]: the EXIF turn is applied, the long side is cut to `maxSide` (null keeps the size), a picture is never enlarged.
 * The decode runs off the main thread and takes its raster from the shared memory budget of the open layer, a refusal is a [MediaException].
 */
object MediaImageDecode {
  suspend fun decode(file: File, maxSide: Int? = null): Bitmap = decodeImage(ImageInput { file.inputStream() }, maxSide, openedAdmit())

  suspend fun decode(uri: Uri, maxSide: Int? = null): Bitmap {
    val environment = ProcessMediaEnvironment.current ?: throw MediaException.Closed()
    val context = checkNotNull(environment.media.binding) { "the layer was not opened with MediaLayer.open(context)" }.factory.appContext
    return decodeImage(ImageInput { context.contentResolver.openInputStream(uri) ?: throw MediaException.Fetch("$uri cannot be opened") }, maxSide, environment.media.decodeAdmit())
  }

  suspend fun decode(bytes: ByteArray, maxSide: Int? = null): Bitmap = decodeImage(ImageInput { ByteArrayInputStream(bytes) }, maxSide, openedAdmit())

  /**
   * The size of an image in memory as [decode] would give it (the EXIF turn applied), read from the header without decoding the pixels.
   * Null when the bytes are no image.
   */
  fun imageSize(bytes: ByteArray): ImageSize? {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }.also { BitmapFactory.decodeByteArray(bytes, 0, bytes.size, it) }
    if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
    val tag = ExifInterface(ByteArrayInputStream(bytes)).getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL)
    val sideways = photoTurn(tag.toUInt()).rotationDegrees % 180u == 90u
    return if (sideways) ImageSize(bounds.outHeight.toUInt(), bounds.outWidth.toUInt()) else ImageSize(bounds.outWidth.toUInt(), bounds.outHeight.toUInt())
  }

  private fun openedAdmit(): DecodeAdmit = (ProcessMediaEnvironment.current ?: throw MediaException.Closed()).media.decodeAdmit()
}

internal fun interface ImageInput {
  fun open(): InputStream
}

internal suspend fun decodeImage(input: ImageInput, maxSide: Int?, admit: DecodeAdmit?): Bitmap =
  withContext(prepareDispatcher) {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }.also { options -> input.open().use { BitmapFactory.decodeStream(it, null, options) } }
    if (bounds.outWidth <= 0 || bounds.outHeight <= 0) throw MediaException.Fetch("the source is not an image")
    val tag = input.open().use { ExifInterface(it).getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL) }
    val plan = imageDecodePlan(bounds.outWidth.toUInt(), bounds.outHeight.toUInt(), tag.toUInt(), maxSide?.coerceAtLeast(0)?.toUInt())
      ?: throw MediaException.Fetch("the image is above the pixel limit")
    val hold = admit?.invoke(plan.cost.toLong())
    try {
      currentCoroutineContext().ensureActive()
      val raster = decodeAt(input, bounds.outWidth, bounds.outHeight, plan.decodeWidth.toInt(), plan.decodeHeight.toInt())
      currentCoroutineContext().ensureActive()
      val matrix = turnMatrix(tag)
      if (matrix.isIdentity) raster else Bitmap.createBitmap(raster, 0, 0, raster.width, raster.height, matrix, true).also { if (it !== raster) raster.recycle() }
    } finally {
      hold?.close()
    }
  }

private fun decodeAt(input: ImageInput, sourceWidth: Int, sourceHeight: Int, width: Int, height: Int): Bitmap {
  var sample = 1
  while (sourceWidth / (sample * 2) >= width && sourceHeight / (sample * 2) >= height) sample *= 2
  val options = BitmapFactory.Options().apply {
    inSampleSize = sample
    val sampledWidth = (sourceWidth + sample - 1) / sample
    if (sampledWidth != width) {
      inScaled = true
      inDensity = sampledWidth
      inTargetDensity = width
    }
  }
  val decoded = input.open().use { BitmapFactory.decodeStream(it, null, options) } ?: throw MediaException.Fetch("the source is not an image")
  if (decoded.width == width && decoded.height == height) return decoded
  return Bitmap.createScaledBitmap(decoded, width, height, true).also { if (it !== decoded) decoded.recycle() }
}

/** The centre square of [source] with a side of [side], cut to a circle. */
internal fun circleOf(source: Bitmap, side: Int): Bitmap {
  val result = Bitmap.createBitmap(side, side, Bitmap.Config.ARGB_8888)
  val scale = side.toFloat() / minOf(source.width, source.height)
  val left = ((side - source.width * scale) / 2).roundToInt()
  val top = ((side - source.height * scale) / 2).roundToInt()
  val canvas = Canvas(result)
  val paint = Paint(Paint.ANTI_ALIAS_FLAG)
  canvas.drawCircle(side / 2f, side / 2f, side / 2f, paint)
  paint.xfermode = PorterDuffXfermode(PorterDuff.Mode.SRC_IN)
  val target = android.graphics.Rect(left, top, left + max(1, (source.width * scale).roundToInt()), top + max(1, (source.height * scale).roundToInt()))
  canvas.drawBitmap(source, null, target, paint)
  return result
}
