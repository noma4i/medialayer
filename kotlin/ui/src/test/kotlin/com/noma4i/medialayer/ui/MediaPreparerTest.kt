package com.noma4i.medialayer.ui

import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.CannotDecode
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaProbeOutcome
import com.noma4i.medialayer.defaultVideoLimits
import com.noma4i.medialayer.MediaPrepareIntent
import com.noma4i.medialayer.MediaPrepareJob
import com.noma4i.medialayer.MediaPrepareOutcome
import com.noma4i.medialayer.mediaPrepareDescriptor
import java.io.File
import java.nio.file.Files
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(MediaLayerTestRunner::class)
class MediaPreparerTest {
  private val context = ApplicationProvider.getApplicationContext<android.content.Context>()
  private val folder = Files.createTempDirectory("prepare").toFile().apply { deleteOnExit() }
  private val fits = VideoFacts(1280, 720, 0, 30f, 1_000_000, 2, true)
  private val bytes = byteArrayOf(1, 2, 3, 4)

  private fun source() = File(folder, "clip.mp4").apply { writeBytes(bytes) }

  private fun descriptor(mirror: Boolean) = mediaPrepareDescriptor(MediaPrepareIntent.Video(null, mirror, CannotDecode.FAIL))

  private fun video(limits: com.noma4i.medialayer.VideoLimits? = null, policy: CannotDecode = CannotDecode.FAIL) = mediaPrepareDescriptor(MediaPrepareIntent.Video(limits, false, policy))

  private fun decoder(maxWidth: Int, maxHeight: Int) = DecoderAbility({ width, height -> width <= maxWidth && height <= maxHeight }, setOf(8), maxWidth, maxHeight)

  private val facts4k = VideoFacts(3840, 2160, 0, 30f, 2_000_000, 2, true, "video/avc", 8)

  private fun preparerOf(facts: VideoFacts, keys: List<String> = emptyList(), decoders: List<DecoderAbility>, exports: MutableList<VideoExport>) =
    MediaPreparer(context, { facts }, { _, _ -> keys }, { decoders }, { exports += it; it.output.writeBytes(byteArrayOf(1)) })

  private fun job(descriptor: ByteArray, attempt: ULong = 1u) = MediaPrepareJob(attempt, source().path, File(folder, "out.mp4").path, descriptor)

  private fun preparer(exporter: suspend (VideoExport) -> Unit = { error("no export expected") }) =
    MediaPreparer(context, { fits }, { _, _ -> emptyList() }, { listOf(DecoderAbility({ _, _ -> true }, emptySet(), 3840, 2160)) }, exporter)

  /** A source that already fits and carries no tag is left alone: the outcome is Unchanged and nothing is written. */
  @Test
  fun passReturnsUnchangedAndWritesNothing() = runBlocking {
    val outcome = preparer().run(job(descriptor(false)), {})

    assertTrue(outcome is MediaPrepareOutcome.Unchanged)
    assertFalse(File(folder, "out.mp4").exists())
    assertEquals(setOf("clip.mp4"), folder.list()!!.toSet())
  }

  /** `cancel` stops the export, the run returns Cancelled and neither the temp file nor the destination stays. */
  @Test
  fun cancelLeavesNoFileAndReturnsCancelled() = runBlocking {
    val started = CompletableDeferred<Unit>()
    val preparer = preparer {
      it.output.writeBytes(byteArrayOf(1))
      started.complete(Unit)
      awaitCancellation()
    }

    val running = async { preparer.run(job(descriptor(true), 9u), {}) }
    withTimeout(10_000) { started.await() }
    preparer.cancel(9u)

    assertEquals(MediaPrepareOutcome.Cancelled, withTimeout(10_000) { running.await() })
    assertFalse(File(folder, "out.mp4").exists())
    assertEquals(setOf("clip.mp4"), folder.list()!!.toSet())
  }

  /** A descriptor of an unknown version is a failure, not a crash, and writes nothing. */
  @Test
  fun unsupportedDescriptorFails() = runBlocking {
    val outcome = preparer().run(job(byteArrayOf(9, 9, 9)), {})

    assertTrue(outcome is MediaPrepareOutcome.Failed)
    assertFalse(File(folder, "out.mp4").exists())
  }

  /** A source above every decoder is refused with the decoder maximum before the export; a source that is only repackaged needs no decoder. */
  @Test
  fun theDecoderIsCheckedOnlyWhenTheVideoIsEncoded() = runBlocking {
    val exports = CopyOnWriteArrayList<VideoExport>()
    val refusing = preparerOf(facts4k, decoders = listOf(decoder(2560, 1440)), exports = exports)

    val refusal = refusing.run(job(video()), {})

    assertEquals("the device decodes video/avc up to 2560x1440, the video is 3840x2160", (refusal as MediaPrepareOutcome.CannotDecode).message)
    assertTrue(exports.isEmpty())
    assertFalse(File(folder, "out.mp4").exists())

    val repacking = preparerOf(fits, keys = listOf("location"), decoders = emptyList(), exports = exports)
    val done = repacking.run(job(video()), {})

    assertEquals(MediaPrepareOutcome.Done(null, "video/mp4"), done)
    assertNull(exports.single().setup.videoMime)
    assertTrue(File(folder, "out.mp4").exists())
  }

  /** The `UploadSource` policy turns a source no decoder can read into Unchanged: nothing is exported or written. */
  @Test
  fun aSourceNoDecoderReadsGoesAsItIsByThePolicy() = runBlocking {
    val exports = CopyOnWriteArrayList<VideoExport>()
    val preparer = preparerOf(facts4k, decoders = listOf(decoder(2560, 1440)), exports = exports)

    val outcome = preparer.run(job(video(policy = CannotDecode.UPLOAD_SOURCE)), {})

    assertEquals(MediaPrepareOutcome.Unchanged(null, "video/mp4"), outcome)
    assertTrue(exports.isEmpty())
    assertFalse(File(folder, "out.mp4").exists())
  }

  /** A source of at most `recodeAboveBytes` goes as it is whatever its codec; a bigger one is encoded. */
  @Test
  fun theByteThresholdKeepsASmallSourceAsItIs() = runBlocking {
    val hevc = VideoFacts(1280, 720, 0, 30f, 1_000_000, 2, false, "video/hevc", null)
    val exports = CopyOnWriteArrayList<VideoExport>()
    val preparer = preparerOf(hevc, decoders = listOf(decoder(3840, 2160)), exports = exports)

    val kept = preparer.run(job(video(defaultVideoLimits().copy(recodeAboveBytes = 1000u))), {})

    assertTrue(kept is MediaPrepareOutcome.Unchanged)
    assertTrue(exports.isEmpty())

    val encoded = preparer.run(job(video(defaultVideoLimits().copy(recodeAboveBytes = 2u))), {})

    assertEquals(MediaPrepareOutcome.Done(null, "video/mp4"), encoded)
    assertEquals("video/avc", exports.single().setup.videoMime)
  }

  /** Settings the planner cannot work with are refused before any descriptor bytes exist. */
  @Test
  fun invalidSettingsGiveNoDescriptor() {
    assertThrows(MediaException.InvalidSpec::class.java) { video(defaultVideoLimits().copy(maxLongSide = 0u)) }
  }

  /** The probe with a descriptor is the planned result, upright; without one it is the file as it is. A turned source is probed upright. */
  @Test
  fun probeGivesThePlannedAndThePlainFacts() = runBlocking {
    val exports = CopyOnWriteArrayList<VideoExport>()
    val path = source().path

    val planned = preparerOf(facts4k, decoders = emptyList(), exports = exports).probe(path, video(), "video/mp4")
    val plain = preparerOf(facts4k, decoders = emptyList(), exports = exports).probe(path, null, "video/mp4")
    val turned = preparerOf(VideoFacts(1920, 1080, 90, 30f, 1_000_000, 2, true), decoders = emptyList(), exports = exports).probe(path, null, "video/mp4")

    assertEquals(MediaProbeOutcome.File(1920u, 1080u, "video/mp4"), planned)
    assertEquals(MediaProbeOutcome.File(3840u, 2160u, "video/mp4"), plain)
    assertEquals(MediaProbeOutcome.File(1080u, 1920u, "video/mp4"), turned)
    assertTrue(preparerOf(facts4k, decoders = emptyList(), exports = exports).probe(File(folder, "missing.mp4").path, null, null) is MediaProbeOutcome.Failed)
    assertTrue(exports.isEmpty())
  }
}
