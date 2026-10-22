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
  suspend fun load(url: String, headers: Map<String, String>): com.noma4i.medialayer.ImageFetchResult
}

internal data class MediaImageData(val url: String, val headers: Map<String, String>, val cacheKey: String, val declared: com.noma4i.medialayer.PixelSize? = null)

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
        val declared = data?.declared
        if (declared == null) return@Interceptor chain.proceed(chain.request)
        val width = (chain.size.width as? coil.size.Dimension.Pixels)?.px?.toDouble() ?: 0.0
        val height = (chain.size.height as? coil.size.Dimension.Pixels)?.px?.toDouble() ?: 0.0
        val pixels = com.noma4i.medialayer.imagePixelSize(width, height, 1.0, declared, chain.request.scale == coil.size.Scale.FILL)
          ?: return@Interceptor chain.proceed(chain.request)
        val request = chain.request.newBuilder().memoryCacheKey(MemoryCache.Key("${data.cacheKey}|${pixels.width}x${pixels.height}|${chain.request.scale}")).build()
        chain.withSize(coil.size.Size(pixels.width.toInt(), pixels.height.toInt())).proceed(request)
      })
      if (Build.VERSION.SDK_INT >= 28) add(ImageDecoderDecoder.Factory()) else add(GifDecoder.Factory())
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
            SourceResult(ImageSource(Buffer().write(result.bytes), options.context), null, source)
          }
      })
    }
    .diskCache(null)
    .diskCachePolicy(CachePolicy.DISABLED)
    .build()
}
