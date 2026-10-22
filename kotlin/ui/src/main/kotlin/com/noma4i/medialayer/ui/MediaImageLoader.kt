package com.noma4i.medialayer.ui

import android.app.ActivityManager
import android.content.Context
import coil.ImageLoader
import coil.decode.DataSource
import coil.decode.ImageSource
import coil.decode.ImageDecoderDecoder
import coil.fetch.Fetcher
import coil.fetch.SourceResult
import coil.key.Keyer
import coil.memory.MemoryCache
import coil.request.CachePolicy
import coil.request.Options
import okio.Buffer

fun interface ImageBytes {
  suspend fun load(url: String, headers: Map<String, String>): com.noma4i.medialayer.ImageFetchResult
}

internal data class MediaImageData(val url: String, val headers: Map<String, String>, val cacheKey: () -> String, val declared: com.noma4i.medialayer.PixelSize? = null, val pixels: com.noma4i.medialayer.PixelSize? = null)

internal fun imageMemoryKey(cacheKey: String, pixels: com.noma4i.medialayer.PixelSize, scale: coil.size.Scale): MemoryCache.Key =
  MemoryCache.Key("$cacheKey|${pixels.width}x${pixels.height}|$scale")

private const val MAX_IMAGE_MEMORY_BYTES: Int = 128 * 1024 * 1024

fun mediaImageLoader(context: Context, bytes: ImageBytes, memoryBytes: Int? = null, stats: com.noma4i.medialayer.ImageCounters = com.noma4i.medialayer.ImageCounters()): ImageLoader {
  val budget = memoryBytes ?: minOf(MAX_IMAGE_MEMORY_BYTES.toLong(), context.getSystemService(ActivityManager::class.java).memoryClass * 1024L * 1024L / 4).toInt()
  require(budget > 0) { "memoryBytes must be positive" }
  return ImageLoader.Builder(context)
    .eventListenerFactory { ImageEvents(stats) }
    .memoryCache(MemoryCache.Builder(context).maxSizeBytes(budget).build())
    .components {
      add(coil.intercept.Interceptor { chain ->
        val data = chain.request.data as? MediaImageData
        if (data == null) return@Interceptor chain.proceed(chain.request)
        val declared = data.declared
        val pixels = declared?.let {
          val width = (chain.size.width as? coil.size.Dimension.Pixels)?.px?.toDouble() ?: 0.0
          val height = (chain.size.height as? coil.size.Dimension.Pixels)?.px?.toDouble() ?: 0.0
          com.noma4i.medialayer.imagePixelSize(width, height, 1.0, it, chain.request.scale == coil.size.Scale.FILL)
        } ?: data.pixels ?: return@Interceptor chain.proceed(chain.request)
        val request = chain.request.newBuilder().memoryCacheKey(imageMemoryKey(data.cacheKey(), pixels, chain.request.scale)).build()
        if (declared == null) chain.proceed(request) else chain.withSize(coil.size.Size(pixels.width.toInt(), pixels.height.toInt())).proceed(request)
      })
      add(ImageDecoderDecoder.Factory())
      add(Keyer<MediaImageData> { data, _ -> data.cacheKey() })
      add(object : Fetcher.Factory<MediaImageData> {
        override fun create(data: MediaImageData, options: Options, imageLoader: ImageLoader): Fetcher =
          Fetcher {
            val result = bytes.load(data.url, data.headers)
            val source = when (result.source) {
              com.noma4i.medialayer.ImageOrigin.NETWORK -> DataSource.NETWORK
              com.noma4i.medialayer.ImageOrigin.CACHE -> DataSource.DISK
              com.noma4i.medialayer.ImageOrigin.LOCAL -> DataSource.MEMORY
            }
            SourceResult(ImageSource(Buffer().write(result.bytes), options.context), null, source)
          }
      })
    }
    .diskCache(null)
    .diskCachePolicy(CachePolicy.DISABLED)
    .build()
}
