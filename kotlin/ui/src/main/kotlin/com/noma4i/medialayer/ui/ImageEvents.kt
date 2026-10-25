package com.noma4i.medialayer.ui

import android.os.Trace
import coil3.BitmapImage
import coil3.EventListener
import coil3.getExtra
import coil3.decode.DataSource
import coil3.decode.DecodeResult
import coil3.decode.Decoder
import coil3.fetch.FetchResult
import coil3.fetch.Fetcher
import coil3.fetch.SourceFetchResult
import coil3.request.ErrorResult
import coil3.request.ImageRequest
import coil3.request.Options
import coil3.request.SuccessResult
import com.noma4i.medialayer.ImageCounters
import com.noma4i.medialayer.ImageWork

internal class ImageEvents(private val stats: ImageCounters) : EventListener() {
  private val cookie = System.identityHashCode(this)
  private var decoding = false

  override fun onStart(request: ImageRequest) {
    stats.recordLater(ImageWork.Request(request.getExtra(PLACEMENT_PIXELS).toULong()))
    Trace.beginAsyncSection("MediaImage.request", cookie)
  }

  override fun decodeStart(request: ImageRequest, decoder: Decoder, options: Options) {
    decoding = true
    Trace.beginAsyncSection("MediaImage.decode", cookie)
  }

  override fun decodeEnd(request: ImageRequest, decoder: Decoder, options: Options, result: DecodeResult?) {
    endDecode()
    val image = result?.image ?: return
    val bitmap = (image as? BitmapImage)?.bitmap
    stats.recordLater(ImageWork.Decode(image.width.coerceAtLeast(0).toUInt(), image.height.coerceAtLeast(0).toUInt(), bitmap?.allocationByteCount?.toULong() ?: 0uL))
  }

  override fun fetchEnd(request: ImageRequest, fetcher: Fetcher, options: Options, result: FetchResult?) {
    if ((result as? SourceFetchResult)?.dataSource == DataSource.DISK && request.data is MediaImageData) stats.recordLater(ImageWork.DiskHit)
  }

  override fun onSuccess(request: ImageRequest, result: SuccessResult) = finish(result.dataSource == DataSource.MEMORY_CACHE)
  override fun onError(request: ImageRequest, result: ErrorResult) = finish(false)

  override fun onCancel(request: ImageRequest) {
    endTrace()
    stats.recordLater(ImageWork.Cancel)
  }

  private fun finish(memoryHit: Boolean) {
    endTrace()
    stats.recordLater(ImageWork.Finish(memoryHit))
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
