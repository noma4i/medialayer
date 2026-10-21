package com.noma4i.medialayer.ui

import android.app.ActivityManager
import android.content.Context
import android.os.Build
import coil.ImageLoader
import coil.decode.DataSource
import coil.decode.ImageSource
import coil.decode.GifDecoder
import coil.decode.ImageDecoderDecoder
import coil.fetch.Fetcher
import coil.fetch.SourceResult
import coil.key.Keyer
import coil.memory.MemoryCache
import coil.request.CachePolicy
import coil.request.Options
import okio.Buffer

fun interface ImageBytes {
  suspend fun load(url: String, headers: Map<String, String>): ByteArray
}

internal data class MediaImageData(val url: String, val headers: Map<String, String>, val cacheKey: String)

private const val MAX_IMAGE_MEMORY_BYTES: Int = 128 * 1024 * 1024

fun mediaImageLoader(context: Context, bytes: ImageBytes, memoryBytes: Int? = null): ImageLoader {
  val budget = memoryBytes ?: minOf(MAX_IMAGE_MEMORY_BYTES.toLong(), context.getSystemService(ActivityManager::class.java).memoryClass * 1024L * 1024L / 4).toInt()
  require(budget > 0) { "memoryBytes must be positive" }
  return ImageLoader.Builder(context)
    .memoryCache(MemoryCache.Builder(context).maxSizeBytes(budget).build())
    .components {
      if (Build.VERSION.SDK_INT >= 28) add(ImageDecoderDecoder.Factory()) else add(GifDecoder.Factory())
      add(Keyer<MediaImageData> { data, _ -> data.cacheKey })
      add(object : Fetcher.Factory<MediaImageData> {
        override fun create(data: MediaImageData, options: Options, imageLoader: ImageLoader): Fetcher =
          Fetcher { SourceResult(ImageSource(Buffer().write(bytes.load(data.url, data.headers)), options.context), null, DataSource.NETWORK) }
      })
    }
    .diskCache(null)
    .diskCachePolicy(CachePolicy.DISABLED)
    .build()
}
