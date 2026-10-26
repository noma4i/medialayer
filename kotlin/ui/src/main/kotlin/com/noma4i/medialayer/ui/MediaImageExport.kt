package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.Bitmap
import coil3.BitmapImage
import coil3.toBitmap
import coil3.ImageLoader
import coil3.request.ErrorResult
import coil3.request.SuccessResult
import coil3.request.allowHardware
import coil3.size.Scale
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.PixelSize
import com.noma4i.medialayer.imagePixelSize
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.isActive
import kotlinx.coroutines.withContext

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
  val pixels = imagePixelSize(width.toDouble(), height.toDouble(), density.toDouble(), declared, true)
  val placement = (width * density).toLong() * (height * density).toLong()
  val request = imageRequest(context, source, pixels, Scale.FILL, { cacheKey(source.url, source.headers) }, placement, admit = viewDecodeAdmit()).newBuilder().allowHardware(false).build()
  pixels?.let { imageLoader.memoryCache?.get(imageMemoryKey(cacheKey(source.url, source.headers), it, Scale.FILL)) }?.let { hit ->
    imageStats().record(com.noma4i.medialayer.ImageWork.Request(placement.toULong()))
    imageStats().record(com.noma4i.medialayer.ImageWork.Finish(true))
    return (hit.image as? BitmapImage)?.bitmap?.softwareCopy()
  }
  val result = try { imageLoader.execute(request) } catch (_: CancellationException) { return null }
  return when (result) {
    is SuccessResult -> result.image.toBitmap()
    is ErrorResult -> null
  }
}

internal suspend fun Bitmap.softwareCopy(copy: (Bitmap) -> Bitmap? = { it.copy(Bitmap.Config.ARGB_8888, false) }): Bitmap? =
  if (config == Bitmap.Config.HARDWARE) withContext(Dispatchers.Default) { copy(this@softwareCopy) } else this
