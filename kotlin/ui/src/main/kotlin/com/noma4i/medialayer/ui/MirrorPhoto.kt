package com.noma4i.medialayer.ui

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Matrix
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** The horizontal mirror of an upright JPEG file, written in place after the EXIF turn is applied. */
internal suspend fun mirrorJpeg(file: File, quality: Int) = withContext(Dispatchers.Default) {
  val source = BitmapFactory.decodeFile(file.path) ?: throw java.io.IOException("The photo cannot be decoded")
  val flipped = Bitmap.createBitmap(source, 0, 0, source.width, source.height, Matrix().apply { preScale(-1f, 1f) }, false)
  try {
    file.outputStream().use { check(flipped.compress(Bitmap.CompressFormat.JPEG, quality, it)) { "The photo cannot be encoded" } }
  } finally {
    if (flipped !== source) flipped.recycle()
    source.recycle()
  }
}
