package com.noma4i.medialayer.ui

import androidx.media3.common.util.UnstableApi
import androidx.media3.container.MdtaMetadataEntry
import androidx.media3.container.Mp4LocationData
import androidx.media3.container.Mp4TimestampData
import com.noma4i.medialayer.videoMetadataToStrip
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.GraphicsMode

@UnstableApi
@GraphicsMode(GraphicsMode.Mode.LEGACY)
@RunWith(MediaLayerTestRunner::class)
@OptIn(ExperimentalCoroutinesApi::class)
class PrepareWritersTest {
  private fun decoder(maxWidth: Int, maxHeight: Int, vararg profiles: Int) =
    DecoderAbility({ width, height -> width <= maxWidth && height <= maxHeight }, profiles.toSet(), maxWidth, maxHeight)

  @Test
  fun inv_send_49_aTenBitSourceWithoutADecoderForItsProfileIsRefused() {
    val tenBit = VideoFacts(1920, 1080, 0, 30f, 2_000_000, 2, false, "video/hevc", 2)

    assertEquals("the device has no video/hevc decoder for profile 2 at 1920x1080", decoderRefusal(tenBit, listOf(decoder(2560, 1440, 1))))
    assertEquals(null, decoderRefusal(tenBit, listOf(decoder(2560, 1440, 1, 2))))
  }

  @Test
  fun inv_send_40_theMuxerKeepsNoMetadataOfTheSourceAndTheCoreNamesEveryEntryItReads() {
    val entries = mutableSetOf<androidx.media3.common.Metadata.Entry>(
      Mp4LocationData(37.33f, -122.03f), Mp4TimestampData(1_700_000_000L, 1_700_000_000L), MdtaMetadataEntry("com.apple.quicktime.make", byteArrayOf(1), 0, 1)
    )
    val keys = entries.mapNotNull(::metadataKeyOf)

    dropSourceMetadata.updateMetadataEntries(entries)

    assertEquals(listOf("location", "creation_time", "com.apple.quicktime.make"), keys)
    assertEquals(keys, videoMetadataToStrip(keys))
    assertEquals(listOf(Mp4TimestampData(0L, 0L)), entries.toList())
    assertTrue(entries.mapNotNull(::metadataKeyOf).isEmpty())
  }

  @Test
  fun inv_send_12_theExifTurnMakesTheCornersOfAPhotoUpright() {
    val corners = floatArrayOf(0f, 0f, 40f, 0f, 40f, 20f, 0f, 20f)
    val expected = mapOf(
      1 to listOf(0f, 0f, 40f, 0f, 40f, 20f, 0f, 20f),
      2 to listOf(40f, 0f, 0f, 0f, 0f, 20f, 40f, 20f),
      3 to listOf(40f, 20f, 0f, 20f, 0f, 0f, 40f, 0f),
      6 to listOf(20f, 0f, 20f, 40f, 0f, 40f, 0f, 0f),
      8 to listOf(0f, 40f, 0f, 0f, 20f, 0f, 20f, 40f)
    )
    for ((tag, points) in expected) {
      val mapped = corners.copyOf().also { turnMatrix(tag).mapPoints(it) }
      val left = mapped.filterIndexed { index, _ -> index % 2 == 0 }.min()
      val top = mapped.filterIndexed { index, _ -> index % 2 == 1 }.min()
      val inBitmap = mapped.mapIndexed { index, value -> Math.round(value - if (index % 2 == 0) left else top).toFloat() }
      assertEquals("tag $tag", points, inBitmap)
    }
  }

  @Test
  fun inv_send_16_theProgressIsReadOnlyWhileTheExportRuns() = runTest {
    withTimeout(10_000) {
      val ticks = AtomicInteger()
      val reported = CopyOnWriteArrayList<Double>()

      val gate = CompletableDeferred<Unit>()
      val runningExport = launch { whileExporting(intervalMs = 20, read = { 0.5 }, report = { reported += it }, onTick = { ticks.incrementAndGet() }) { gate.await() } }
      runCurrent()
      advanceTimeBy(60)
      runCurrent()
      assertTrue(ticks.get() >= 3)
      gate.complete(Unit)
      runningExport.join()
      val afterCompletion = ticks.get()
      advanceTimeBy(1_000)
      runCurrent()

      assertTrue(afterCompletion >= 3)
      assertEquals(afterCompletion, ticks.get())
      assertTrue(reported.all { it == 0.5 })

      val running = launch { whileExporting(intervalMs = 20, read = { null }, report = {}, onTick = { ticks.incrementAndGet() }) { awaitCancellation() } }
      runCurrent()
      advanceTimeBy(100)
      runCurrent()
      running.cancelAndJoin()
      val afterCancel = ticks.get()
      advanceTimeBy(1_000)
      runCurrent()

      assertTrue(afterCancel > afterCompletion)
      assertEquals(afterCancel, ticks.get())
    }
  }
}
