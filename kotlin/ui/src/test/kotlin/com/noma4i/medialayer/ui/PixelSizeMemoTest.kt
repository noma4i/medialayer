package com.noma4i.medialayer.ui

import com.noma4i.medialayer.PixelSize
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Assert.assertEquals
import org.junit.Test

class PixelSizeMemoTest {
  @Test
  fun inv_image_40_tilesOfOneSizeCostOneImagePixelSizeCallAndTheMapStaysBounded() {
    val calls = AtomicInteger()
    val memo = PixelSizeMemo { width, _, _, _, _ -> calls.incrementAndGet(); PixelSize(width.toUInt(), 1u) }

    repeat(30) { assertEquals(PixelSize(100u, 1u), memo.size(100.0, 100.0, 2.0, null, true)) }
    assertEquals(1, calls.get())
    assertEquals(null, PixelSizeMemo { _, _, _, _, _ -> null }.size(1.0, 1.0, 1.0, null, true))

    for (width in 1..70) memo.size(width.toDouble() + 1000, 1.0, 1.0, null, true)
    val before = calls.get()
    memo.size(100.0, 100.0, 2.0, null, true)
    assertEquals(before + 1, calls.get())
  }
}
