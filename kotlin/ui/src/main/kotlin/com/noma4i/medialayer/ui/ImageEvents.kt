package com.noma4i.medialayer.ui

import android.graphics.drawable.BitmapDrawable
import android.os.Trace
import coil.EventListener
import coil.decode.DataSource
import coil.decode.DecodeResult
import coil.decode.Decoder
import coil.fetch.FetchResult
import coil.fetch.Fetcher
import coil.fetch.SourceResult
import coil.request.ErrorResult
import coil.request.ImageRequest
import coil.request.Options
import coil.request.SuccessResult
import com.noma4i.medialayer.ImageCounters
import com.noma4i.medialayer.ImageWork

internal class ImageEvents(private val stats: ImageCounters) : EventListener {
  private val cookie = System.identityHashCode(this)
  private var decoding = false

  override fun onStart(request: ImageRequest) {
    stats.record(ImageWork.Request((request.parameters.value<Long>("placementPixels") ?: 0L).toULong()))
    Trace.beginAsyncSection("MediaImage.request", cookie)
  }

  override fun decodeStart(request: ImageRequest, decoder: Decoder, options: Options) {
    decoding = true
    Trace.beginAsyncSection("MediaImage.decode", cookie)
  }

  override fun decodeEnd(request: ImageRequest, decoder: Decoder, options: Options, result: DecodeResult?) {
    endDecode()
    val drawable = result?.drawable ?: return
    val bitmap = (drawable as? BitmapDrawable)?.bitmap
    stats.record(ImageWork.Decode(drawable.intrinsicWidth.coerceAtLeast(0).toUInt(), drawable.intrinsicHeight.coerceAtLeast(0).toUInt(), bitmap?.allocationByteCount?.toULong() ?: 0uL))
  }

  override fun fetchEnd(request: ImageRequest, fetcher: Fetcher, options: Options, result: FetchResult?) {
    if ((result as? SourceResult)?.dataSource == DataSource.DISK && request.data is MediaImageData) stats.record(ImageWork.DiskHit)
  }

  override fun onSuccess(request: ImageRequest, result: SuccessResult) = finish(result.dataSource == DataSource.MEMORY_CACHE)
  override fun onError(request: ImageRequest, result: ErrorResult) = finish(false)

  override fun onCancel(request: ImageRequest) {
    endTrace()
    stats.record(ImageWork.Cancel((request.data as? MediaImageData)?.cacheKey ?: request.data.toString()))
  }

  private fun finish(memoryHit: Boolean) {
    endTrace()
    stats.record(ImageWork.Finish(memoryHit))
  }

  private fun endTrace() {
    endDecode()
    Trace.endAsyncSection("MediaImage.request", cookie)
  }

  private fun endDecode() {
    if (!decoding) return
    decoding = false
    Trace.endAsyncSection("MediaImage.decode", cookie)
  }
}
