package com.noma4i.medialayer.ui

import android.app.ActivityManager
import android.content.Context
import androidx.compose.ui.graphics.asImageBitmap
import coil3.ImageLoader
import coil3.decode.DataSource
import coil3.decode.ImageSource
import coil3.gif.AnimatedImageDecoder
import coil3.fetch.Fetcher
import coil3.fetch.SourceFetchResult
import coil3.memory.MemoryCache
import coil3.request.CachePolicy
import coil3.request.Options
import coil3.request.SuccessResult
import java.io.ByteArrayInputStream
import okio.buffer
import okio.source

fun interface ImageBytes {
  suspend fun load(url: String, headers: Map<String, String>): com.noma4i.medialayer.ImageFetchResult
}

internal data class MediaImageData(val url: String, val headers: Map<String, String>, val media: com.noma4i.medialayer.MediaLayer, val declared: com.noma4i.medialayer.PixelSize? = null, val pixels: com.noma4i.medialayer.PixelSize? = null, val rest: RestFrame? = null, val repeats: Repeats = Repeats.Forever, val dimensions: PixelSizeMemo.Args? = null, val rated: Boolean = false)

internal fun imageMemoryKey(cacheKey: String, pixels: com.noma4i.medialayer.PixelSize, scale: coil3.size.Scale, rest: RestFrame? = null, repeats: Repeats = Repeats.Forever, rated: Boolean = false): MemoryCache.Key =
  MemoryCache.Key("$cacheKey|${pixels.width}x${pixels.height}|$scale" + (rest?.let { "|rest:$it" } ?: if (repeats == Repeats.Forever) "" else "|repeats:$repeats") + if (rated) "|rated" else "")

private const val MAX_IMAGE_MEMORY_BYTES: Int = 128 * 1024 * 1024

internal class StillImages : coil3.intercept.Interceptor {
  private data class Source(val url: String, val headers: Map<String, String>)
  private data class Entry(val key: MemoryCache.Key, val area: Long, val pixels: com.noma4i.medialayer.PixelSize?, val dimensions: PixelSizeMemo.Args?, val scale: coil3.size.Scale)
  private val entries = object : LinkedHashMap<Source, List<Entry>>(512, 0.75f, true) {
    override fun removeEldestEntry(eldest: MutableMap.MutableEntry<Source, List<Entry>>) = size > 512
  }

  override suspend fun intercept(chain: coil3.intercept.Interceptor.Chain) = chain.proceed()

  fun record(data: MediaImageData, result: SuccessResult) {
    if (data.rest != null || data.repeats != Repeats.Forever || data.rated || result.image !is coil3.BitmapImage) return
    val key = result.memoryCacheKey ?: return
    val area = result.image.width.toLong() * result.image.height
    val source = Source(data.url, data.headers.toMap())
    synchronized(entries) {
      entries[source] = (listOf(Entry(key, area, data.pixels, data.dimensions, result.request.scale)) + entries[source].orEmpty().filterNot { it.key == key }).take(4)
    }
  }

  fun keys(url: String, headers: Map<String, String>, pixels: com.noma4i.medialayer.PixelSize?, dimensions: PixelSizeMemo.Args?, scale: coil3.size.Scale): List<MemoryCache.Key> = synchronized(entries) {
    entries[Source(url, headers)].orEmpty().filterNot { it.pixels == pixels && it.dimensions == dimensions && it.scale == scale }.sortedByDescending { it.area }.map { it.key }
  }
}

internal fun ImageLoader.interim(url: String, headers: Map<String, String>, pixels: com.noma4i.medialayer.PixelSize?, dimensions: PixelSizeMemo.Args?, scale: coil3.size.Scale): androidx.compose.ui.graphics.ImageBitmap? =
  components.interceptors.filterIsInstance<StillImages>().firstOrNull()?.keys(url, headers, pixels, dimensions, scale)?.firstNotNullOfOrNull {
    (memoryCache?.get(it)?.image as? coil3.BitmapImage)?.bitmap?.asImageBitmap()
  }

fun mediaImageLoader(context: Context, bytes: ImageBytes, memoryBytes: Int? = null, stats: com.noma4i.medialayer.ImageCounters = com.noma4i.medialayer.ImageCounters()): ImageLoader =
  mediaImageLoader(context, bytes, memoryBytes, stats, PixelSizeMemo())

internal fun mediaImageLoader(context: Context, bytes: ImageBytes, memoryBytes: Int?, stats: com.noma4i.medialayer.ImageCounters, pixelSizes: PixelSizeMemo, admit: DecodeAdmit? = null): ImageLoader {
  val budget = memoryBytes ?: minOf(MAX_IMAGE_MEMORY_BYTES.toLong(), context.getSystemService(ActivityManager::class.java).memoryClass * 1024L * 1024L / 4).toInt()
  val stillImages = StillImages()
  require(budget > 0) { "memoryBytes must be positive" }
  return ImageLoader.Builder(context)
    .eventListenerFactory { ImageEvents(stats) }
    .memoryCache(MemoryCache.Builder().maxSizeBytes(budget.toLong()).build())
    .components {
      add(stillImages)
      admit?.let { add(AdmittedDecoders(it)) }
      add(coil3.intercept.Interceptor { chain ->
        val data = chain.request.data as? MediaImageData
        if (data == null) return@Interceptor chain.proceed()
        val dimensions = data.dimensions ?: data.declared?.let {
          val width = (chain.size.width as? coil3.size.Dimension.Pixels)?.px?.toDouble() ?: 0.0
          val height = (chain.size.height as? coil3.size.Dimension.Pixels)?.px?.toDouble() ?: 0.0
          PixelSizeMemo.Args(width, height, 1.0, it, chain.request.scale == coil3.size.Scale.FILL)
        }
        val prepared = pixelSizes.prepare(PixelSizeMemo.Input(PixelSizeMemo.Source(data.url, data.headers, data.media), dimensions, data.pixels)) { data.media.cacheKey(data.url, data.headers) }
        val pixels = prepared.pixels
        val request = chain.request.newBuilder()
        request.memoryCacheKey(if (pixels == null) MemoryCache.Key(prepared.identity + if (data.rated) "|rated" else "") else imageMemoryKey(prepared.identity, pixels, chain.request.scale, data.rest, data.repeats, data.rated))
        val keyed = chain.withRequest(request.build())
        val result = if (pixels == null) keyed.proceed() else keyed.withSize(coil3.size.Size(pixels.width.toInt(), pixels.height.toInt())).proceed()
        (result as? SuccessResult)?.let { pixelSizes.retain(it.image, prepared); stillImages.record(data, it) }
        result
      })
      add(AnimatedImageDecoder.Factory())
      add(object : Fetcher.Factory<MediaImageData> {
        override fun create(data: MediaImageData, options: Options, imageLoader: ImageLoader): Fetcher =
          Fetcher {
            if (isBundledUrl(data.url)) return@Fetcher SourceFetchResult(ImageSource(openBundled(context, data.url).source().buffer(), options.fileSystem), null, DataSource.MEMORY)
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
