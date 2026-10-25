package com.noma4i.medialayer.ui

import com.noma4i.medialayer.PixelSize
import com.noma4i.medialayer.imagePixelSize

internal class PixelSizeMemo(private val compute: (Double, Double, Double, PixelSize?, Boolean) -> PixelSize? = ::imagePixelSize) {
  private data class Args(val width: Double, val height: Double, val scale: Double, val declared: PixelSize?, val crop: Boolean)

  private val sizes = object : LinkedHashMap<Args, PixelSize?>() {
    override fun removeEldestEntry(eldest: MutableMap.MutableEntry<Args, PixelSize?>) = size > CAPACITY
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
