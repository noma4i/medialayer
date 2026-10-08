package com.noma4i.medialayer.ui

import android.content.Context
import coil.ImageLoader
import coil.decode.DataSource
import coil.decode.ImageSource
import coil.fetch.Fetcher
import coil.fetch.SourceResult
import coil.key.Keyer
import coil.request.CachePolicy
import coil.request.Options
import okio.Buffer

fun interface ImageBytes {
  suspend fun load(url: String, headers: Map<String, String>): ByteArray
}

internal data class MediaImageData(val url: String, val headers: Map<String, String>)

fun mediaImageLoader(context: Context, bytes: ImageBytes): ImageLoader =
  ImageLoader.Builder(context)
    .components {
      add(Keyer<MediaImageData> { data, _ -> data.url })
      add(object : Fetcher.Factory<MediaImageData> {
        override fun create(data: MediaImageData, options: Options, imageLoader: ImageLoader): Fetcher =
          Fetcher { SourceResult(ImageSource(Buffer().write(bytes.load(data.url, data.headers)), options.context), null, DataSource.NETWORK) }
      })
    }
    .diskCache(null)
    .diskCachePolicy(CachePolicy.DISABLED)
    .build()
