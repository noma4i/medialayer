package com.noma4i.medialayer.ui

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
}
