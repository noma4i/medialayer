package com.noma4i.medialayer.ui

import com.noma4i.medialayer.normalizeItem
import com.noma4i.medialayer.toItem
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Rect
import android.graphics.RectF
import android.util.Size
import android.graphics.Matrix
import android.media.ExifInterface
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.CropFractions
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.PhotoEdit
import com.noma4i.medialayer.PhotoPlan
import com.noma4i.medialayer.PixelSize
import com.noma4i.medialayer.jpegQuality
import com.noma4i.medialayer.photoPlan
import com.noma4i.medialayer.photoTurn
import java.io.File
import java.util.UUID
import kotlin.math.roundToInt
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext

/**
 * A photo made upright by its EXIF orientation and saved as JPEG. No metadata is copied, so the editor tag and every EXIF field are gone.
 * [maxSize] fits the result into the rectangle (never enlarged), [outputSize] gives the exact size and wins over [maxSize],
 * [crop] is a rectangle in fractions 0 to 1 of the upright photo (origin top left), [mirror] flips the cropped result horizontally.
 * An edit that holds no pixel, such as an empty crop or a size of zero, is refused with [MediaException.Fetch]. The core plans every size.
 */
suspend fun MediaLayer.prepareImage(
  source: MediaSource,
  quality: Double = 0.9,
  maxSize: Size? = null,
  crop: RectF? = null,
  outputSize: Size? = null,
  mirror: Boolean = false
): PreparedMedia {
  val cache = checkNotNull(binding) { "the layer was not opened with MediaLayer.open(context)" }.factory.appContext.cacheDir
  val edit = PhotoEdit(
    maxSize?.toPixels(),
    crop?.let { CropFractions(it.left.toDouble(), it.top.toDouble(), (it.right - it.left).toDouble(), (it.bottom - it.top).toDouble()) },
    outputSize?.toPixels(),
    mirror
  )
  return prepareImageAt(cache, preparableFile(normalizeItem(source.toItem()).url), quality, decodeAdmit(), edit)
}

private fun Size.toPixels() = PixelSize(width.coerceAtLeast(0).toUInt(), height.coerceAtLeast(0).toUInt())

internal val plainPhoto = PhotoEdit(null, null, null, false)

internal suspend fun prepareImageAt(cache: File, file: File, quality: Double, admit: DecodeAdmit, edit: PhotoEdit = plainPhoto): PreparedMedia =
  withContext(prepareDispatcher) { PreparedMedia(upright(file, quality, cache, admit, edit), true) }

internal var photoOrientation: (File) -> Int = { ExifInterface(it.path).getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL) }

private suspend fun upright(file: File, quality: Double, cache: File, admit: DecodeAdmit, edit: PhotoEdit): File {
  val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }.also { BitmapFactory.decodeFile(file.path, it) }
  if (bounds.outWidth <= 0 || bounds.outHeight <= 0) throw MediaException.Fetch("${file.path} is not an image")
  val tag = photoOrientation(file)
  val plan = photoPlan(bounds.outWidth.toUInt(), bounds.outHeight.toUInt(), tag.toUInt(), edit)
    ?: throw MediaException.Fetch("${file.path} is above the pixel limit or the edit holds no pixel")
  return admit(plan.cost.toLong()).use { decodeUpright(file, quality, cache, turnMatrix(tag), plan) }
}

private suspend fun decodeUpright(file: File, quality: Double, cache: File, matrix: Matrix, plan: PhotoPlan): File {
  val source = BitmapFactory.decodeFile(file.path) ?: throw MediaException.Fetch("${file.path} is not an image")
  currentCoroutineContext().ensureActive()
  val turned = Bitmap.createBitmap(source, 0, 0, source.width, source.height, matrix, true)
  val drawn = if (plan.needsDraw) redraw(turned, plan) else turned
  val output = File(cache, "prepared-${UUID.randomUUID()}.jpg")
  try {
    currentCoroutineContext().ensureActive()
    output.outputStream().use { check(drawn.compress(Bitmap.CompressFormat.JPEG, (jpegQuality(quality) * 100).roundToInt(), it)) { "the photo does not encode" } }
    return output
  } catch (error: Throwable) {
    output.delete()
    throw error
  } finally {
    if (turned !== source) source.recycle()
    if (drawn !== turned) turned.recycle()
    drawn.recycle()
  }
}

/** The crop of the plan drawn into the output size, flipped when the plan asks for the mirror. */
internal fun redraw(upright: Bitmap, plan: PhotoPlan): Bitmap {
  val width = plan.output.width.toInt()
  val height = plan.output.height.toInt()
  val result = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
  val canvas = Canvas(result)
  if (plan.mirror) canvas.scale(-1f, 1f, width / 2f, 0f)
  val area = Rect(plan.crop.x.toInt(), plan.crop.y.toInt(), (plan.crop.x + plan.crop.width).toInt(), (plan.crop.y + plan.crop.height).toInt())
  canvas.drawBitmap(upright, area, Rect(0, 0, width, height), Paint(Paint.FILTER_BITMAP_FLAG))
  return result
}

internal fun turnMatrix(exifOrientation: Int): Matrix {
  val turn = photoTurn(exifOrientation.toUInt())
  return Matrix().apply {
    postRotate(turn.rotationDegrees.toFloat())
    if (turn.flipHorizontal) postScale(-1f, 1f)
  }
}
