package com.noma4i.medialayer.ui

import com.noma4i.medialayer.PixelSize
import com.noma4i.medialayer.imagePixelSize
import coil3.Image
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.lang.ref.WeakReference
import java.util.WeakHashMap

internal class PixelSizeMemo(private val images: MutableMap<Image, Prepared> = WeakHashMap(), private val compute: (Double, Double, Double, PixelSize?, Boolean) -> PixelSize? = ::imagePixelSize) {
  internal data class Args(val width: Double, val height: Double, val scale: Double, val declared: PixelSize?, val crop: Boolean)
  internal class Source(val url: String, val headers: Map<String, String>, owner: Any) {
    private val ownerRef = WeakReference(owner)
    private val ownerHash = System.identityHashCode(owner)

    override fun equals(other: Any?): Boolean {
      if (this === other) return true
      if (other !is Source) return false
      val owner = ownerRef.get() ?: return false
      return owner === other.ownerRef.get() && url == other.url && headers == other.headers
    }

    override fun hashCode() = 31 * (31 * url.hashCode() + headers.hashCode()) + ownerHash
  }
  internal data class Input(val source: Source, val dimensions: Args?, val pixels: PixelSize?)
  internal class Prepared(val input: Input, val identity: String, val pixels: PixelSize?)
  private class Identity(val source: WeakReference<Source>, val value: String)
  private val requests = WeakHashMap<Input, WeakReference<Prepared>>()
  private val sources = WeakHashMap<Source, Identity>()

  private val sizes = object : LinkedHashMap<Args, PixelSize?>() {
    override fun removeEldestEntry(eldest: MutableMap.MutableEntry<Args, PixelSize?>) = size > CAPACITY
  }

  fun retain(image: Image, prepared: Prepared) {
    synchronized(requests) { images[image] = prepared }
  }

  suspend fun prepare(input: Input, readKey: () -> String): Prepared {
    synchronized(requests) { requests[input]?.get()?.let { return it } }
    return withContext(Dispatchers.Default) {
      val identity = synchronized(requests) { sources[input.source]?.value } ?: readKey()
      val args = input.dimensions
      val pixels = if (args == null) input.pixels else size(args.width, args.height, args.scale, args.declared, args.crop)
      synchronized(requests) {
        requests[input]?.get()?.let { return@withContext it }
        val previous = sources[input.source]
        val canonical = input.copy(source = previous?.source?.get() ?: input.source)
        val prepared = Prepared(canonical, previous?.value ?: identity, pixels)
        requests[canonical] = WeakReference(prepared)
        sources[canonical.source] = Identity(WeakReference(canonical.source), prepared.identity)
        prepared
      }
    }
  }

  fun size(width: Double, height: Double, scale: Double, declared: PixelSize?, crop: Boolean): PixelSize? {
    val args = Args(width, height, scale, declared, crop)
    synchronized(sizes) { if (sizes.containsKey(args)) return sizes[args] }
    val size = compute(width, height, scale, declared, crop)
    synchronized(sizes) { sizes[args] = size }
    return size
  }

  private companion object {
    const val CAPACITY = 64
  }
}
