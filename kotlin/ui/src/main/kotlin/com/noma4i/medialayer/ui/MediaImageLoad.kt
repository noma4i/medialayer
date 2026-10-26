package com.noma4i.medialayer.ui

import android.graphics.Bitmap
import com.noma4i.medialayer.Fetcher
import com.noma4i.medialayer.ImageFetchResult
import com.noma4i.medialayer.ImageLoadRequest
import com.noma4i.medialayer.loadImage
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

object MediaImageLoad {
  const val DEFAULT_CACHE_BYTES: Long = 8L shl 20
  const val DEFAULT_TIMEOUT_MS: Long = 5_000

  suspend fun load(
    url: String,
    directory: File,
    headers: Map<String, String> = emptyMap(),
    cacheBytes: Long = DEFAULT_CACHE_BYTES,
    timeoutMs: Long = DEFAULT_TIMEOUT_MS,
    fetcher: Fetcher = OkHttpFetcher(timeoutMs = timeoutMs)
  ): ImageFetchResult {
    val request = ImageLoadRequest(url, headers, directory.absolutePath, cacheBytes.toULong(), timeoutMs.toULong())
    return withContext(Dispatchers.IO) { loadImage(request, fetcher) }
  }

  /**
   * The picture at [url] as an upright [Bitmap] with the long side cut to [side]. With [circle] the result is a square of [side] pixels
   * cut to a circle. Works without an open layer, the decode takes its raster from the shared memory budget when a layer is open.
   */
  suspend fun loadBitmap(
    url: String,
    directory: File,
    side: Int,
    circle: Boolean = false,
    headers: Map<String, String> = emptyMap(),
    cacheBytes: Long = DEFAULT_CACHE_BYTES,
    timeoutMs: Long = DEFAULT_TIMEOUT_MS,
    fetcher: Fetcher = OkHttpFetcher(timeoutMs = timeoutMs)
  ): Bitmap {
    val fetched = load(url, directory, headers, cacheBytes, timeoutMs, fetcher)
    val admit = ProcessMediaEnvironment.current?.media?.decodeAdmit()
    return bitmapOf(fetched.bytes, side, circle, admit)
  }

  internal suspend fun bitmapOf(bytes: ByteArray, side: Int, circle: Boolean, admit: DecodeAdmit?): Bitmap {
    val decoded = decodeImage(ImageInput { java.io.ByteArrayInputStream(bytes) }, side, admit)
    if (!circle) return decoded
    return withContext(Dispatchers.Default) { circleOf(decoded, side).also { decoded.recycle() } }
  }
}
