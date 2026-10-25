package com.noma4i.medialayer

import android.content.Context
import android.net.Uri
import java.io.File

/**
 * A source declared by value, without a class of its own. The core cleans the text once: a blank url, cover or blur hash is absent.
 * A source without a url is still created, its place fails with `MediaState.Failed`.
 */
class MediaSourceValue(
  url: String,
  override val kind: Kind,
  override val headers: Map<String, String> = emptyMap(),
  cover: String? = null,
  blurHash: String? = null,
  override val size: Pair<Int, Int>? = null,
  override val strategy: CacheStrategy = CacheStrategy.WINDOW,
) : MediaSource {
  private val clean = normalizeItem(MediaItem(kind, url, headers, cover, blurHash, strategy = strategy))
  override val url: String = clean.url
  override val cover: String? = clean.cover
  override val blurHash: String? = clean.blurHash

  private val parts get() = listOf(url, kind, headers, cover, blurHash, size, strategy)

  override fun equals(other: Any?): Boolean = other is MediaSourceValue && parts == other.parts

  override fun hashCode(): Int = parts.hashCode()

  override fun toString(): String = "MediaSourceValue($url, $kind)"

  companion object {
    /** A local file by absolute path or `file://` text. */
    fun file(path: String, kind: Kind, cover: String? = null, blurHash: String? = null, size: Pair<Int, Int>? = null, strategy: CacheStrategy = CacheStrategy.WINDOW): MediaSourceValue =
      MediaSourceValue(localFileUrl(path), kind, cover = cover, blurHash = blurHash, size = size, strategy = strategy)

    fun file(file: File, kind: Kind, cover: String? = null, blurHash: String? = null, size: Pair<Int, Int>? = null, strategy: CacheStrategy = CacheStrategy.WINDOW): MediaSourceValue =
      file(file.absolutePath, kind, cover, blurHash, size, strategy)

    fun file(uri: Uri, kind: Kind, cover: String? = null, blurHash: String? = null, size: Pair<Int, Int>? = null, strategy: CacheStrategy = CacheStrategy.WINDOW): MediaSourceValue =
      file(uri.toString(), kind, cover, blurHash, size, strategy)

    /** An `http` or `https` url. */
    fun remote(url: String, kind: Kind, headers: Map<String, String> = emptyMap(), cover: String? = null, blurHash: String? = null, size: Pair<Int, Int>? = null, strategy: CacheStrategy = CacheStrategy.WINDOW): MediaSourceValue =
      MediaSourceValue(remoteUrl(url), kind, headers, cover, blurHash, size, strategy)

    /** A video or an audio in `res/raw`. An image is a programming error, the text comes from the core. */
    fun raw(context: Context, id: Int, kind: Kind, cover: String? = null, blurHash: String? = null, size: Pair<Int, Int>? = null, strategy: CacheStrategy = CacheStrategy.WINDOW): MediaSourceValue {
      bundledSourceError(kind)?.let { throw IllegalArgumentException(it) }
      return MediaSourceValue("android.resource://${context.packageName}/$id", kind, cover = cover, blurHash = blurHash, size = size, strategy = strategy)
    }

    /** A video or an audio in `assets`. An image is a programming error, the text comes from the core. */
    fun asset(name: String, kind: Kind, cover: String? = null, blurHash: String? = null, size: Pair<Int, Int>? = null, strategy: CacheStrategy = CacheStrategy.WINDOW): MediaSourceValue {
      bundledSourceError(kind)?.let { throw IllegalArgumentException(it) }
      return MediaSourceValue("asset:///${name.trim().trimStart('/')}", kind, cover = cover, blurHash = blurHash, size = size, strategy = strategy)
    }
  }
}
