package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.Bitmap
import android.media.MediaMetadataRetriever
import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.PlaybackRoute
import com.noma4i.medialayer.TonePixels
import com.noma4i.medialayer.TonePreparation
import coil3.BitmapImage
import coil3.ImageLoader
import coil3.request.ImageRequest
import coil3.request.SuccessResult
import coil3.size.Scale
import coil3.size.Precision
import coil3.request.CachePolicy
import coil3.request.allowHardware
import java.lang.ref.WeakReference
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.withContext

internal fun interface FrameSource {
  suspend fun first(route: PlaybackRoute): Bitmap?
}

internal suspend fun <T> cancellableBlocking(abort: () -> Unit, block: () -> T): T = coroutineScope {
  val work = async(Dispatchers.IO) { block() }
  try {
    work.await()
  } catch (cancel: CancellationException) {
    abort()
    throw cancel
  }
}

internal object MetadataFrames : FrameSource {
  override suspend fun first(route: PlaybackRoute): Bitmap? {
    val retriever = MediaMetadataRetriever()
    return try {
      cancellableBlocking(abort = retriever::release) {
        try {
          retriever.setDataSource(route.url, route.headers)
          retriever.getScaledFrameAtTime(0, MediaMetadataRetriever.OPTION_CLOSEST_SYNC, ToneFrames.LONG_SIDE, ToneFrames.LONG_SIDE)
        } catch (_: RuntimeException) {
          null
        }
      }
    } finally {
      retriever.release()
    }
  }
}

internal class ToneFrames(media: MediaLayer, private val context: Context, private val loader: ImageLoader, private val frames: FrameSource = MetadataFrames) : TonePreparation {
  private val media = WeakReference(media)

  override suspend fun prepare(source: MediaItem): TonePixels? = withContext(Dispatchers.IO) {
    val media = media.get() ?: return@withContext null
    if (!media.tonePixelsNeeded(source)) return@withContext null
    try {
      val bitmap = if (source.kind == Kind.IMAGE || source.cover != null) {
        image(media, source)
      } else if (source.kind == Kind.VIDEO) {
        frames.first(media.playUrl(source))
      } else null
      if (bitmap == null) return@withContext null
      try {
        pixels(bitmap)?.let { TonePixels(it.width.toUInt(), it.height.toUInt(), it.rgba) }
      } finally { bitmap.recycle() }
    } catch (error: CancellationException) {
      throw error
    } catch (_: Exception) { null }
  }

  private suspend fun image(media: MediaLayer, source: MediaItem): Bitmap? {
    val url = source.cover ?: source.url
    val request = ImageRequest.Builder(context)
      .data(MediaImageData(url, source.headers, media))
      .size(LONG_SIDE, LONG_SIDE)
      .scale(Scale.FIT)
      .precision(Precision.EXACT)
      .allowHardware(false)
      .memoryCachePolicy(CachePolicy.DISABLED)
      .build()
    val result = loader.execute(request) as? SuccessResult ?: return null
    return (result.image as? BitmapImage)?.bitmap
  }

  companion object {
    const val LONG_SIDE = 64

    class Pixels(val width: Int, val height: Int, val rgba: ByteArray)

    fun pixels(bitmap: Bitmap): Pixels? {
      val long = maxOf(bitmap.width, bitmap.height)
      if (bitmap.width <= 0 || bitmap.height <= 0) return null

      val scale = minOf(1.0, LONG_SIDE.toDouble() / long)
      val width = maxOf(1, Math.round(bitmap.width * scale).toInt())
      val height = maxOf(1, Math.round(bitmap.height * scale).toInt())
      if (bitmap.config == Bitmap.Config.HARDWARE) return null
      val small = Bitmap.createScaledBitmap(bitmap, width, height, true)
      val argb = IntArray(width * height)
      small.getPixels(argb, 0, width, 0, 0, width, height)
      if (small !== bitmap) small.recycle()
      val rgba = ByteArray(argb.size * 4)
      argb.forEachIndexed { index, color ->
        rgba[index * 4] = (color shr 16).toByte()
        rgba[index * 4 + 1] = (color shr 8).toByte()
        rgba[index * 4 + 2] = color.toByte()
        rgba[index * 4 + 3] = (color ushr 24).toByte()
      }
      return Pixels(width, height, rgba)
    }
  }
}
