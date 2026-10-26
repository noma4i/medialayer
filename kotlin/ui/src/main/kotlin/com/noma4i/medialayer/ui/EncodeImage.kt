package com.noma4i.medialayer.ui

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.media.ExifInterface
import com.noma4i.medialayer.ImageFormat
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.imageEncodePlan
import com.noma4i.medialayer.jpegQuality
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.File
import java.util.UUID
import kotlin.math.roundToInt
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext

/**
 * An upright [Bitmap] saved as a JPEG or PNG file. A JPEG has no transparency, so it is flattened onto white. [quality] (0 to 1) is for JPEG only.
 * The file belongs to the caller. The memory is taken from the shared budget of the layer.
 */
suspend fun MediaLayer.encodeImage(bitmap: Bitmap, format: ImageFormat = ImageFormat.JPEG, quality: Double = 0.9): PreparedMedia {
  val cache = checkNotNull(binding) { "the layer was not opened with MediaLayer.open(context)" }.factory.appContext.cacheDir
  return encodeBitmapAt(cache, bitmap, format, quality, decodeAdmit())
}

/** The bytes of an image (any format the system decodes, such as WebP) encoded again as JPEG or PNG and turned upright by their EXIF orientation. */
suspend fun MediaLayer.encodeImage(bytes: ByteArray, format: ImageFormat = ImageFormat.JPEG, quality: Double = 0.9): ByteArray =
  encodeBytes(bytes, format, quality, decodeAdmit())

private val MAY_HAVE_ALPHA = setOf("image/png", "image/webp", "image/gif")

internal suspend fun encodeBitmapAt(cache: File, bitmap: Bitmap, format: ImageFormat, quality: Double, admit: DecodeAdmit): PreparedMedia =
  withContext(prepareDispatcher) {
    val data = encodeBitmap(bitmap, ExifInterface.ORIENTATION_NORMAL, format, quality, admit)
    val output = File(cache, "encoded-${UUID.randomUUID()}.${if (format == ImageFormat.PNG) "png" else "jpg"}")
    try {
      output.writeBytes(data)
    } catch (error: Throwable) {
      output.delete()
      throw error
    }
    PreparedMedia(output, true)
  }

internal suspend fun encodeBytes(bytes: ByteArray, format: ImageFormat, quality: Double, admit: DecodeAdmit): ByteArray =
  withContext(prepareDispatcher) {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }.also { BitmapFactory.decodeByteArray(bytes, 0, bytes.size, it) }
    if (bounds.outWidth <= 0 || bounds.outHeight <= 0) throw MediaException.Fetch("the bytes are not an image")
    val tag = ExifInterface(ByteArrayInputStream(bytes)).getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL)
    val plan = imageEncodePlan(bounds.outWidth.toUInt(), bounds.outHeight.toUInt(), tag.toUInt(), bounds.outMimeType in MAY_HAVE_ALPHA, format)
      ?: throw MediaException.Fetch("the image has no pixel or is above the pixel limit")
    admit(plan.cost.toLong()).use {
      val decoded = BitmapFactory.decodeByteArray(bytes, 0, bytes.size) ?: throw MediaException.Fetch("the bytes are not an image")
      try {
        encodeOwn(decoded, tag, format, quality, plan.flatten)
      } finally {
        decoded.recycle()
      }
    }
  }

private suspend fun encodeBitmap(bitmap: Bitmap, tag: Int, format: ImageFormat, quality: Double, admit: DecodeAdmit): ByteArray {
  val plan = imageEncodePlan(bitmap.width.toUInt(), bitmap.height.toUInt(), tag.toUInt(), bitmap.hasAlpha(), format)
    ?: throw MediaException.Fetch("the image has no pixel or is above the pixel limit")
  return admit(plan.cost.toLong()).use { encodeOwn(bitmap, tag, format, quality, plan.flatten) }
}

private suspend fun encodeOwn(source: Bitmap, tag: Int, format: ImageFormat, quality: Double, flatten: Boolean): ByteArray {
  currentCoroutineContext().ensureActive()
  val matrix = turnMatrix(tag)
  val turned = if (matrix.isIdentity) source else Bitmap.createBitmap(source, 0, 0, source.width, source.height, matrix, true)
  val flat = if (flatten) onWhite(turned) else turned
  try {
    currentCoroutineContext().ensureActive()
    val out = ByteArrayOutputStream()
    val compressed = if (format == ImageFormat.PNG) flat.compress(Bitmap.CompressFormat.PNG, 100, out)
    else flat.compress(Bitmap.CompressFormat.JPEG, (jpegQuality(quality) * 100).roundToInt(), out)
    check(compressed) { "the image does not encode" }
    return out.toByteArray()
  } finally {
    if (flat !== turned) flat.recycle()
    if (turned !== source) turned.recycle()
  }
}

private fun onWhite(source: Bitmap): Bitmap {
  val result = Bitmap.createBitmap(source.width, source.height, Bitmap.Config.ARGB_8888)
  Canvas(result).apply {
    drawColor(Color.WHITE)
    drawBitmap(source, 0f, 0f, null)
  }
  return result
}
