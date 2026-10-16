package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.Bitmap
import androidx.core.graphics.drawable.toBitmap
import coil.ImageLoader
import coil.request.ErrorResult
import coil.request.SuccessResult
import coil.size.Scale
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaState
import com.noma4i.medialayer.PixelSize
import com.noma4i.medialayer.imagePixelSize
import com.noma4i.medialayer.toItem
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.isActive

suspend fun MediaLayer.image(
  context: Context,
  source: MediaSource,
  width: Float,
  height: Float,
  density: Float,
  imageLoader: ImageLoader = checkNotNull(binding?.imageLoader) { "Open the layer with MediaLayer.open or pass an imageLoader" }
): Bitmap? {
  if (!currentCoroutineContext().isActive) return null
  val declared = source.size?.let { (w, h) -> PixelSize(w.toUInt(), h.toUInt()) }
  val pixels = declared ?: imagePixelSize(width.toDouble(), height.toDouble(), density.toDouble(), null)
  val request = imageRequest(context, source, pixels, Scale.FILL).newBuilder().allowHardware(false).build()
  request.memoryCacheKey?.let { imageLoader.memoryCache?.get(it) }?.let { hit ->
    itemState(source.url, MediaState.Ready)
    return hit.bitmap.softwareCopy()
  }
  itemState(source.url, MediaState.Loading)
  val result = try { imageLoader.execute(request) } catch (_: CancellationException) { return null }
  return when (result) {
    is SuccessResult -> {
      val bitmap = result.drawable.toBitmap()
      itemState(source.url, MediaState.Ready)
      ToneFrames.submit(this, source.toItem(), bitmap)
      bitmap
    }
    is ErrorResult -> {
      imageFailure(result.throwable)?.let { itemState(source.url, it) }
      null
    }
  }
}

private fun Bitmap.softwareCopy(): Bitmap? = if (config == Bitmap.Config.HARDWARE) copy(Bitmap.Config.ARGB_8888, false) else this
