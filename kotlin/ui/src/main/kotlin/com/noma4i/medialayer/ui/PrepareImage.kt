package com.noma4i.medialayer.ui

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Matrix
import android.media.ExifInterface
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.jpegQuality
import com.noma4i.medialayer.photoTurn
import java.io.File
import java.util.UUID
import kotlin.math.roundToInt
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext

/** A photo made upright by its EXIF orientation and saved as JPEG. No metadata is copied, so the editor tag and every EXIF field are gone. */
suspend fun MediaLayer.prepareImage(source: MediaSource, quality: Double = 0.9): PreparedMedia {
  val cache = checkNotNull(binding) { "the layer was not opened with MediaLayer.open(context)" }.factory.appContext.cacheDir
  return prepareImageAt(cache, preparableFile(source.url), quality)
}

internal suspend fun prepareImageAt(cache: File, file: File, quality: Double): PreparedMedia =
  withContext(prepareDispatcher) { PreparedMedia(upright(file, quality, cache), true) }

private suspend fun upright(file: File, quality: Double, cache: File): File {
  val tag = ExifInterface(file.path).getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL)
  val source = BitmapFactory.decodeFile(file.path) ?: throw MediaException.Fetch("${file.path} is not an image")
  currentCoroutineContext().ensureActive()
  val turned = Bitmap.createBitmap(source, 0, 0, source.width, source.height, turnMatrix(tag), true)
  val output = File(cache, "prepared-${UUID.randomUUID()}.jpg")
  try {
    currentCoroutineContext().ensureActive()
    output.outputStream().use { check(turned.compress(Bitmap.CompressFormat.JPEG, (jpegQuality(quality) * 100).roundToInt(), it)) { "the photo does not encode" } }
    return output
  } catch (error: Throwable) {
    output.delete()
    throw error
  } finally {
    if (turned !== source) source.recycle()
    turned.recycle()
  }
}

internal fun turnMatrix(exifOrientation: Int): Matrix {
  val turn = photoTurn(exifOrientation.toUInt())
  return Matrix().apply {
    postRotate(turn.rotationDegrees.toFloat())
    if (turn.flipHorizontal) postScale(-1f, 1f)
  }
}
