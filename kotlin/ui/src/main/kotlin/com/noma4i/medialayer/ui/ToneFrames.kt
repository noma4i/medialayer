package com.noma4i.medialayer.ui

import android.graphics.Bitmap
import android.graphics.drawable.BitmapDrawable
import android.graphics.drawable.Drawable
import android.media.MediaMetadataRetriever
import android.os.Build
import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.PlaybackRoute
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

internal fun interface FrameSource {
  fun first(route: PlaybackRoute): Bitmap?
}

internal object MetadataFrames : FrameSource {
  override fun first(route: PlaybackRoute): Bitmap? {
    val retriever = MediaMetadataRetriever()
    return try {
      retriever.setDataSource(route.url, route.headers)
      if (Build.VERSION.SDK_INT >= 27) {
        retriever.getScaledFrameAtTime(0, MediaMetadataRetriever.OPTION_CLOSEST_SYNC, ToneFrames.LONG_SIDE, ToneFrames.LONG_SIDE)
      } else {
        retriever.getFrameAtTime(0, MediaMetadataRetriever.OPTION_CLOSEST_SYNC)
      }
    } catch (_: RuntimeException) {
      null
    } finally {
      retriever.release()
    }
  }
}

internal object ToneFrames {
  const val LONG_SIDE = 64

  class Pixels(val width: Int, val height: Int, val rgba: ByteArray)

  fun pixels(bitmap: Bitmap): Pixels? {
    val long = maxOf(bitmap.width, bitmap.height)
    if (bitmap.width <= 0 || bitmap.height <= 0) return null

    val scale = minOf(1.0, LONG_SIDE.toDouble() / long)
    val width = maxOf(1, Math.round(bitmap.width * scale).toInt())
    val height = maxOf(1, Math.round(bitmap.height * scale).toInt())
    val small = shrink(bitmap, width, height) ?: return null
    val argb = IntArray(width * height)
    small.getPixels(argb, 0, width, 0, 0, width, height)
    val rgba = ByteArray(argb.size * 4)
    argb.forEachIndexed { index, color ->
      rgba[index * 4] = (color shr 16).toByte()
      rgba[index * 4 + 1] = (color shr 8).toByte()
      rgba[index * 4 + 2] = color.toByte()
      rgba[index * 4 + 3] = (color ushr 24).toByte()
    }
    return Pixels(width, height, rgba)
  }

  private fun shrink(bitmap: Bitmap, width: Int, height: Int): Bitmap? {
    val scaled = try {
      Bitmap.createScaledBitmap(bitmap, width, height, true)
    } catch (_: RuntimeException) {
      val software = bitmap.copy(Bitmap.Config.ARGB_8888, false) ?: return null
      Bitmap.createScaledBitmap(software, width, height, true).also { if (it !== software) software.recycle() }
    }
    return if (scaled.config == Bitmap.Config.ARGB_8888) scaled else scaled.copy(Bitmap.Config.ARGB_8888, false)
  }

  suspend fun submit(media: MediaLayer, source: MediaItem, bitmap: Bitmap?) {
    if (bitmap == null) return

    withContext(Dispatchers.Default) {
      val pixels = try { pixels(bitmap) } catch (_: RuntimeException) { null } ?: return@withContext
      media.submitTonePixels(source, pixels.width.toUInt(), pixels.height.toUInt(), pixels.rgba)
    }
  }

  suspend fun submit(media: MediaLayer, source: MediaItem, drawable: Drawable?) = submit(media, source, (drawable as? BitmapDrawable)?.bitmap)

  suspend fun submitFirstFrame(media: MediaLayer, source: MediaItem, route: PlaybackRoute?, frames: FrameSource) {
    if (source.cover != null || route == null) return

    submit(media, source, withContext(Dispatchers.IO) { frames.first(route) })
  }
}
