package com.noma4i.medialayer.ui

import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.CannotDecode
import com.noma4i.medialayer.MediaPrepareIntent
import com.noma4i.medialayer.MediaPrepareJob
import com.noma4i.medialayer.MediaPrepareOutcome
import com.noma4i.medialayer.mediaPrepareDescriptor
import java.io.File
import java.nio.file.Files
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
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
}
