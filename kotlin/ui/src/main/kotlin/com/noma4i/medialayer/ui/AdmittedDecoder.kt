package com.noma4i.medialayer.ui

import android.graphics.BitmapFactory
import coil3.ImageLoader
import coil3.decode.DecodeResult
import coil3.decode.Decoder
import coil3.fetch.SourceFetchResult
import coil3.request.Options
import coil3.size.Dimension
import com.noma4i.medialayer.decodeCost

internal class AdmittedDecoder(private val inner: Decoder, private val admit: DecodeAdmit, private val bytes: Long) : Decoder {
  override suspend fun decode(): DecodeResult? {
    val hold = admit(bytes)
    try {
      return inner.decode()
    } finally {
      hold.close()
    }
  }
}

internal class AdmittedDecoders(private val admit: DecodeAdmit, private val inner: Decoder.Factory? = null) : Decoder.Factory {
  override fun create(result: SourceFetchResult, options: Options, imageLoader: ImageLoader): Decoder? {
    val decoder = inner?.create(result, options, imageLoader) ?: imageLoader.components.newDecoder(result, options, imageLoader, 1)?.first ?: return null
    return AdmittedDecoder(decoder, admit, decodeBytes(result, options))
  }
}

internal fun decodeBytes(result: SourceFetchResult, options: Options): Long {
  val width = (options.size.width as? Dimension.Pixels)?.px
  val height = (options.size.height as? Dimension.Pixels)?.px
  val (w, h) = if (width != null && height != null) width to height else {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    BitmapFactory.decodeStream(result.source.source().peek().inputStream(), null, bounds)
    bounds.outWidth to bounds.outHeight
  }
  return decodeCost(w.coerceAtLeast(0).toUInt(), h.coerceAtLeast(0).toUInt())?.toLong()
    ?: throw IllegalArgumentException("the image size cannot be read or is above the pixel limit")
}
