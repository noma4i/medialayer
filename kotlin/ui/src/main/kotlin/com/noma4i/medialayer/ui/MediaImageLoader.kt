package com.noma4i.medialayer.ui

import android.app.ActivityManager
import android.content.Context
import coil3.ImageLoader
import coil3.decode.DataSource
import coil3.decode.ImageSource
import coil3.gif.AnimatedImageDecoder
import coil3.fetch.Fetcher
import coil3.fetch.SourceFetchResult
import coil3.key.Keyer
import coil3.memory.MemoryCache
import coil3.request.CachePolicy
import coil3.request.Options
import java.io.ByteArrayInputStream
import okio.buffer
import okio.source

fun interface ImageBytes {
  suspend fun load(url: String, headers: Map<String, String>): com.noma4i.medialayer.ImageFetchResult
}

internal data class MediaImageData(val url: String, val headers: Map<String, String>, private val keyOf: () -> String, val declared: com.noma4i.medialayer.PixelSize? = null, val pixels: com.noma4i.medialayer.PixelSize? = null, val rest: RestFrame? = null, val repeats: Repeats = Repeats.Forever) {
  val cacheKey: String by lazy(keyOf)
}

internal fun imageMemoryKey(cacheKey: String, pixels: com.noma4i.medialayer.PixelSize, scale: coil3.size.Scale, rest: RestFrame? = null, repeats: Repeats = Repeats.Forever): MemoryCache.Key =
  MemoryCache.Key("$cacheKey|${pixels.width}x${pixels.height}|$scale" + (rest?.let { "|rest:$it" } ?: if (repeats == Repeats.Forever) "" else "|repeats:$repeats"))

private const val MAX_IMAGE_MEMORY_BYTES: Int = 128 * 1024 * 1024

fun mediaImageLoader(context: Context, bytes: ImageBytes, memoryBytes: Int? = null, stats: com.noma4i.medialayer.ImageCounters = com.noma4i.medialayer.ImageCounters()): ImageLoader =
  mediaImageLoader(context, bytes, memoryBytes, stats, PixelSizeMemo())

internal fun mediaImageLoader(context: Context, bytes: ImageBytes, memoryBytes: Int?, stats: com.noma4i.medialayer.ImageCounters, pixelSizes: PixelSizeMemo): ImageLoader {
  val budget = memoryBytes ?: minOf(MAX_IMAGE_MEMORY_BYTES.toLong(), context.getSystemService(ActivityManager::class.java).memoryClass * 1024L * 1024L / 4).toInt()
  require(budget > 0) { "memoryBytes must be positive" }
  return ImageLoader.Builder(context)
    .eventListenerFactory { ImageEvents(stats) }
    .memoryCache(MemoryCache.Builder().maxSizeBytes(budget.toLong()).build())
    .components {
      add(coil3.intercept.Interceptor { chain ->
        val data = chain.request.data as? MediaImageData
        if (data == null) return@Interceptor chain.proceed()
        val declared = data.declared
        val pixels = declared?.let {
          val width = (chain.size.width as? coil3.size.Dimension.Pixels)?.px?.toDouble() ?: 0.0
          val height = (chain.size.height as? coil3.size.Dimension.Pixels)?.px?.toDouble() ?: 0.0
          pixelSizes.size(width, height, 1.0, it, chain.request.scale == coil3.size.Scale.FILL)
        } ?: data.pixels ?: return@Interceptor chain.proceed()
        val keyed = chain.withRequest(chain.request.newBuilder().memoryCacheKey(imageMemoryKey(data.cacheKey, pixels, chain.request.scale, data.rest, data.repeats)).build())
        if (declared == null) keyed.proceed() else keyed.withSize(coil3.size.Size(pixels.width.toInt(), pixels.height.toInt())).proceed()
      })
      add(AnimatedImageDecoder.Factory())
      add(Keyer<MediaImageData> { data, _ -> data.cacheKey })
      add(object : Fetcher.Factory<MediaImageData> {
        override fun create(data: MediaImageData, options: Options, imageLoader: ImageLoader): Fetcher =
          Fetcher {
            val result = bytes.load(data.url, data.headers)
            val source = when (result.source) {
              com.noma4i.medialayer.ImageOrigin.NETWORK -> DataSource.NETWORK
              com.noma4i.medialayer.ImageOrigin.CACHE -> DataSource.DISK
              com.noma4i.medialayer.ImageOrigin.LOCAL -> DataSource.MEMORY
            }
            SourceFetchResult(ImageSource(ByteArrayInputStream(result.bytes).source().buffer(), options.fileSystem), null, source)
          }
      })
    }
    .diskCache(null)
    .diskCachePolicy(CachePolicy.DISABLED)
    .build()
}
