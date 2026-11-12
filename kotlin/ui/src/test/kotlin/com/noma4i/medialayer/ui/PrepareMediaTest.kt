package com.noma4i.medialayer.ui

import android.app.Application
import android.os.Looper
import androidx.media3.common.util.UnstableApi
import androidx.media3.container.MdtaMetadataEntry
import androidx.media3.container.Mp4LocationData
import androidx.media3.container.Mp4TimestampData
import com.noma4i.medialayer.videoMetadataToStrip
import androidx.media3.common.audio.ChannelMixingAudioProcessor
import androidx.media3.effect.FrameDropEffect
import androidx.media3.effect.Presentation
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.VideoLimits
import com.noma4i.medialayer.defaultVideoLimits
import com.noma4i.medialayer.planVideo
import java.io.File
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicInteger
import kotlin.coroutines.CoroutineContext
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Before
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.GraphicsMode

@UnstableApi
@GraphicsMode(GraphicsMode.Mode.LEGACY)
@RunWith(MediaLayerTestRunner::class)
class PrepareMediaTest {
  private val context = ApplicationProvider.getApplicationContext<Application>()
  private val readerBefore = videoReader
  private val exporterBefore = videoExporter
  private val metadataBefore = metadataReader
  private val decoderBefore = decoderReader
  private val dispatcherBefore = prepareDispatcher

  @Before
  fun everyDecoderTakesEverySource() {
    decoderReader = { listOf(decoder(Int.MAX_VALUE, Int.MAX_VALUE)) }
  }

  @After
  fun restore() {
    videoReader = readerBefore
    videoExporter = exporterBefore
    metadataReader = metadataBefore
    decoderReader = decoderBefore
    prepareDispatcher = dispatcherBefore
  }

  private fun small(): VideoLimits = defaultVideoLimits().copy(maxLongSide = 48u)

  private fun turnedClip() = VideoFacts(64, 32, 90, 10f, 17_120, null, false)

  private fun fittingClip() = VideoFacts(1280, 720, 0, 30f, 1_500_000, 2, true)

  private fun source() = File.createTempFile("clip", ".mp4").apply { writeBytes(byteArrayOf(0)); deleteOnExit() }

  private class Counting(private val inner: CoroutineDispatcher, val dispatched: AtomicInteger = AtomicInteger()) : CoroutineDispatcher() {
    override fun dispatch(context: CoroutineContext, block: Runnable) {
      dispatched.incrementAndGet()
      inner.dispatch(context, block)
    }
  }

  @Test
  fun inv_send_17_thePlanBecomesTheEncoderSetup() {
    val limits = small()
    val facts = turnedClip()
    val plan = planVideo(facts.toSource(), limits)

    val setup = transformSetup(plan, limits, facts)

    assertEquals(listOf(true), setup.videoEffects.map { it is Presentation })
    assertEquals(plan.bitrate.toInt(), setup.videoBitrate)
    assertEquals(128_000, setup.audioBitrate)
    assertEquals("video/avc", setup.videoMime)
    assertEquals("audio/mp4a-latm", setup.audioMime)
    assertEquals(0, setup.audioProcessors.count { it is ChannelMixingAudioProcessor })
  }

  @Test
  fun inv_send_17_aFastClipDropsFramesAndAMonoClipIsMixedToStereo() {
    val limits = defaultVideoLimits()
    val facts = VideoFacts(1280, 720, 0, 120f, 6_000_000, 1, true)

    val setup = transformSetup(planVideo(facts.toSource(), limits), limits, facts)

    assertEquals(1, setup.videoEffects.count { it is FrameDropEffect })
    assertEquals(0, setup.videoEffects.count { it is Presentation })
    assertEquals(1, setup.audioProcessors.count { it is ChannelMixingAudioProcessor })
  }

  private fun decoder(maxWidth: Int, maxHeight: Int, vararg profiles: Int) =
    DecoderAbility({ width, height -> width <= maxWidth && height <= maxHeight }, profiles.toSet(), maxWidth, maxHeight)

  @Test
  fun inv_send_48_aSourceLargerThanEveryDecoderIsRefusedWithTheDecoderMaximumBeforeTheExport() = runBlocking {
    val exports = CopyOnWriteArrayList<VideoExport>()
    videoReader = { VideoFacts(3840, 2160, 0, 30f, 2_000_000, 2, false, "video/avc", 8) }
    decoderReader = { listOf(decoder(2560, 1440, 8), decoder(1920, 1088, 8)) }
    videoExporter = { exports += it }

    val refusal = assertThrows(MediaException.Fetch::class.java) { runBlocking { prepareVideoAt(context, source(), defaultVideoLimits(), null) } }

    assertTrue(refusal.message.orEmpty(), refusal.message.orEmpty().contains("the device decodes video/avc up to 2560x1440, the video is 3840x2160"))
    assertTrue(exports.isEmpty())
  }

  @Test
  fun inv_send_50_aSourceThatNeedsNoDecoderIsCopiedWithoutTheDecoderCheck() = runBlocking {
    val exports = CopyOnWriteArrayList<VideoExport>()
    videoReader = { VideoFacts(3840, 2160, 0, 30f, 2_000_000, 2, true, "video/avc", 8) }
    metadataReader = { _, _ -> listOf("major_brand", "location") }
    decoderReader = { listOf(decoder(2560, 1440, 8)) }
    videoExporter = { request ->
      exports += request
      request.output.writeBytes(byteArrayOf(1))
    }
    val limits = defaultVideoLimits().copy(maxLongSide = 3840u)

    val prepared = prepareVideoAt(context, source(), limits, null)

    assertTrue(prepared.isNew)
    assertEquals(1, exports.size)
    prepared.file.delete()
    Unit
  }

  @Test
  fun inv_send_49_aTenBitSourceWithoutADecoderForItsProfileIsRefused() {
    val tenBit = VideoFacts(1920, 1080, 0, 30f, 2_000_000, 2, false, "video/hevc", 2)

    assertEquals("the device has no video/hevc decoder for profile 2 at 1920x1080", decoderRefusal(tenBit, listOf(decoder(2560, 1440, 1))))
    assertEquals(null, decoderRefusal(tenBit, listOf(decoder(2560, 1440, 1, 2))))
  }

  @Test
  fun inv_send_06_aClipThatFitsIsHandedBackAndNeverEncoded() = runBlocking {
    val file = source()
    videoReader = { fittingClip() }
    metadataReader = { _, _ -> listOf("major_brand", "handler_name") }
    videoExporter = { error("a fitting clip must not be encoded") }

    val prepared = prepareVideoAt(context, file, defaultVideoLimits(), null)

    assertSame(file, prepared.file)
    assertFalse(prepared.isNew)
  }

  @Test
  fun inv_send_39_aClipThatFitsButCarriesLocationIsCopiedWithoutEncodingAndWithoutMetadata() = runBlocking {
    val file = source()
    val exports = CopyOnWriteArrayList<VideoExport>()
    videoReader = { fittingClip() }
    metadataReader = { _, _ -> listOf("major_brand", "location", "com.apple.quicktime.make") }
    videoExporter = { request ->
      exports += request
      request.output.writeBytes(byteArrayOf(1))
    }

    val prepared = prepareVideoAt(context, file, defaultVideoLimits(), null)

    val setup = exports.single().setup
    assertTrue(prepared.isNew)
    assertEquals(exports.single().output, prepared.file)
    assertEquals(listOf(null, null), listOf(setup.videoMime, setup.audioMime))
    assertTrue(setup.videoEffects.isEmpty() && setup.audioProcessors.isEmpty())
    prepared.file.delete()
    Unit
  }

  @Test
  fun inv_send_40_theMuxerKeepsNoMetadataOfTheSourceAndTheCoreNamesEveryEntryItReads() {
    val entries = mutableSetOf<androidx.media3.common.Metadata.Entry>(
      Mp4LocationData(37.33f, -122.03f), Mp4TimestampData(1_700_000_000L, 1_700_000_000L), MdtaMetadataEntry("com.apple.quicktime.make", byteArrayOf(1), 0, 1)
    )
    val keys = entries.map(::metadataKeyOf)

    dropSourceMetadata.updateMetadataEntries(entries)

    assertEquals(listOf("location", "creation_time", "com.apple.quicktime.make"), keys)
    assertEquals(keys, videoMetadataToStrip(keys))
    assertTrue(entries.isEmpty())
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
  fun inv_send_13_cancellingStopsTheExportAndLeavesNoFile() = runBlocking {
    val file = source()
    val started = Job()
    val outputs = CopyOnWriteArrayList<File>()
    videoReader = { turnedClip() }
    videoExporter = { request ->
      request.output.writeBytes(byteArrayOf(1))
      outputs += request.output
      started.complete()
      awaitCancellation()
    }

    val job = launch { prepareVideoAt(context, file, small(), null) }
    withTimeout(10_000) { started.join() }
    job.cancelAndJoin()

    assertEquals(1, outputs.size)
    assertFalse(outputs.single().exists())
  }

  @Test
  fun inv_send_14_theWorkDoesNotRunOnTheMainThread() = runBlocking {
    val counting = Counting(Dispatchers.IO)
    prepareDispatcher = counting
    val threads = CopyOnWriteArrayList<Boolean>()
    videoReader = { threads += (Looper.myLooper() == Looper.getMainLooper()); turnedClip() }
    videoExporter = { it.output.writeBytes(byteArrayOf(1)) }

    val video = prepareVideoAt(context, source(), small(), null)

    assertEquals(listOf(false), threads)
    assertTrue(counting.dispatched.get() >= 1)
    video.file.delete()
    Unit
  }

  @Test
  fun inv_send_15_aSourceThatIsNotALocalFileIsRefused() {
    val real = source().path
    for (url in listOf("https://o.test$real", "asset://$real", "file:///tmp/${java.util.UUID.randomUUID()}.mp4")) {
      assertThrows(url, MediaException.Fetch::class.java) { preparableFile(url) }
    }
    assertEquals(real, preparableFile("file://$real").path)
  }

  @Test
  fun inv_send_16_theProgressIsReadOnlyWhileTheExportRuns() = runBlocking {
    withTimeout(10_000) {
      val ticks = AtomicInteger()
      val reported = CopyOnWriteArrayList<Double>()

      whileExporting(intervalMs = 20, read = { 0.5 }, report = { reported += it }, onTick = { ticks.incrementAndGet() }) { delay(130) }
      val afterCompletion = ticks.get()
      delay(200)

      assertTrue(afterCompletion >= 3)
      assertEquals(afterCompletion, ticks.get())
      assertTrue(reported.all { it == 0.5 })

      val running = launch { whileExporting(intervalMs = 20, read = { null }, report = {}, onTick = { ticks.incrementAndGet() }) { awaitCancellation() } }
      delay(100)
      running.cancelAndJoin()
      val afterCancel = ticks.get()
      delay(200)

      assertTrue(afterCancel > afterCompletion)
      assertEquals(afterCancel, ticks.get())
    }
  }
}
