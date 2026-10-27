package com.noma4i.medialayer.ui

import com.noma4i.medialayer.PixelSize
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertSame
import org.junit.Test

class PixelSizeMemoTest {
  @Test
  fun inv_image_05_aLiveImageRetainsItsRequestPreparation() = kotlinx.coroutines.runBlocking {
    val owners = java.util.WeakHashMap<coil3.Image, PixelSizeMemo.Prepared>()
    val memo = PixelSizeMemo(images = owners)
    val image = object : coil3.Image {
      override val width = 16
      override val height = 16
      override val size = 1024L
      override val shareable = true
      override fun draw(canvas: coil3.Canvas) = error("The metadata test does not draw")
    }
    val source = PixelSizeMemo.Source("https://o.test/owned.png", emptyMap(), memo)
    val prepared = memo.prepare(PixelSizeMemo.Input(source, null, PixelSize(16u, 16u))) { source.url }
    val reference = java.lang.ref.WeakReference(prepared)

    memo.retain(image, prepared)

    val live = reference.get()
    assertNotNull(live)
    assertSame(live, owners[image])
  }

  @Test
  fun inv_image_34_samePixelSizeKeepsTheIdentityOfTheSource() = kotlinx.coroutines.runBlocking {
    val keys = AtomicInteger()
    val memo = PixelSizeMemo { _, _, _, _, _ -> PixelSize(56u, 56u) }
    data class Example(val owner: Any, val url: String, val side: Double, val identity: String, val reads: Int)
    val first = Any()
    val second = Any()
    val url = "https://o.test/layout.png"
    val examples = listOf(
      Example(first, url, 50.0, "$url#ns:a", 1),
      Example(first, url, 51.0, "$url#ns:a", 1),
      Example(first, "https://o.test/changed.png", 51.0, "https://o.test/changed.png", 2),
      Example(second, url, 51.0, "$url#ns:b", 3)
    )
    val prepared = mutableListOf<PixelSizeMemo.Prepared>()
    for (example in examples) {
      val source = PixelSizeMemo.Source(example.url, mapOf("Authorization" to "token"), example.owner)
      prepared += memo.prepare(PixelSizeMemo.Input(source, PixelSizeMemo.Args(example.side, example.side, 1.0, null, true), null)) { keys.incrementAndGet(); example.identity }
      assertEquals(example.reads, keys.get())
    }
    assertEquals(examples.map { it.identity }, prepared.map { it.identity })
  }

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
