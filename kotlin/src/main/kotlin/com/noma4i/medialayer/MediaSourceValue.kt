package com.noma4i.medialayer

import android.content.Context
import android.net.Uri
import java.io.File

/**
 * A source declared by value, without a class of its own. The constructors keep the text as given and call nothing native.
 * The core cleans it once when the place is declared: a text that is no source url, a blank cover or blur hash is absent, and a place without a url fails with `MediaState.Failed`.
 */
class MediaSourceValue(
  override val url: String,
  override val kind: Kind,
  override val headers: Map<String, String> = emptyMap(),
  override val cover: String? = null,
  override val blurHash: String? = null,
  override val size: Pair<Int, Int>? = null,
  override val strategy: CacheStrategy = CacheStrategy.WINDOW,
) : MediaSource {
  private val parts get() = listOf(url, kind, headers, cover, blurHash, size, strategy)

  override fun equals(other: Any?): Boolean = other is MediaSourceValue && parts == other.parts

  override fun hashCode(): Int = parts.hashCode()

  override fun toString(): String = "MediaSourceValue($url, $kind)"

  companion object {
    /** A local file by absolute path or `file://` text, the core refuses any other text. */
    fun file(path: String, kind: Kind, cover: String? = null, blurHash: String? = null, size: Pair<Int, Int>? = null, strategy: CacheStrategy = CacheStrategy.WINDOW): MediaSourceValue =
      MediaSourceValue(path, kind, cover = cover, blurHash = blurHash, size = size, strategy = strategy)

    fun file(file: File, kind: Kind, cover: String? = null, blurHash: String? = null, size: Pair<Int, Int>? = null, strategy: CacheStrategy = CacheStrategy.WINDOW): MediaSourceValue =
      file(file.absolutePath, kind, cover, blurHash, size, strategy)

    fun file(uri: Uri, kind: Kind, cover: String? = null, blurHash: String? = null, size: Pair<Int, Int>? = null, strategy: CacheStrategy = CacheStrategy.WINDOW): MediaSourceValue =
      file(uri.toString(), kind, cover, blurHash, size, strategy)

    /** An `http` or `https` url, the core refuses any other text. */
    fun remote(url: String, kind: Kind, headers: Map<String, String> = emptyMap(), cover: String? = null, blurHash: String? = null, size: Pair<Int, Int>? = null, strategy: CacheStrategy = CacheStrategy.WINDOW): MediaSourceValue =
      MediaSourceValue(url, kind, headers, cover, blurHash, size, strategy)

    /** A video or an audio in `res/raw`. An image is a programming error, the text comes from the core. */
    fun raw(context: Context, id: Int, kind: Kind, cover: String? = null, blurHash: String? = null, size: Pair<Int, Int>? = null, strategy: CacheStrategy = CacheStrategy.WINDOW): MediaSourceValue {
      if (kind == Kind.IMAGE) throw IllegalArgumentException(bundledSourceError(kind))
      return MediaSourceValue("android.resource://${context.packageName}/$id", kind, cover = cover, blurHash = blurHash, size = size, strategy = strategy)
    }

    /** A video or an audio in `assets`. An image is a programming error, the text comes from the core. */
    fun asset(name: String, kind: Kind, cover: String? = null, blurHash: String? = null, size: Pair<Int, Int>? = null, strategy: CacheStrategy = CacheStrategy.WINDOW): MediaSourceValue {
      if (kind == Kind.IMAGE) throw IllegalArgumentException(bundledSourceError(kind))
      return MediaSourceValue("asset:///${name.trim().trimStart('/')}", kind, cover = cover, blurHash = blurHash, size = size, strategy = strategy)
    }
  }
}
