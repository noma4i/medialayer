package com.noma4i.medialayer.ui

import android.media.AudioFormat
import android.media.MediaRecorder
import com.noma4i.medialayer.NativeFailure
import com.noma4i.medialayer.VoiceEncoderSettings
import java.io.File
import java.nio.ByteBuffer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(MediaLayerTestRunner::class)
class VoiceTakeTest {
  private val settings = VoiceEncoderSettings(44_100u, 1u, 128_000u)
  private val spec = PcmSpec(MediaRecorder.AudioSource.MIC, 44_100, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
  private val file = File.createTempFile("voice-take", ".m4a").also { it.delete() }

  /** The writer boundary: it records the calls and creates a file only when the take opens it, like the real one; the take never creates one. */
  private class Writer(val failOpen: Boolean = false) : VoiceFileWriter {
    val calls = ArrayList<String>()
    var done: ((Result<Long>) -> Unit)? = null
    lateinit var target: File
    override fun open(file: File, settings: VoiceEncoderSettings) {
      calls += "open"
      if (failOpen) throw IllegalStateException("writer is not ready")
      target = file.also { it.writeText("m4a") }
    }
    override fun append(chunk: PcmChunk) { calls += "append" }
    override fun finish(done: (Result<Long>) -> Unit) { calls += "finish"; this.done = done }
    override fun cancel() { calls += "cancel" }
  }

  private class Outcomes : VoiceTakeListener {
    val events = ArrayList<String>()
    override fun onStarted() { events += "started" }
    override fun onLimit() { events += "limit" }
    override fun onFinished(file: File, durationUs: Long) { events += "finished:$durationUs" }
    override fun onFailed(failure: NativeFailure) { events += "failed" }
    override fun onCancelled() { events += "cancelled" }
    override fun onLost() { events += "lost" }
  }

  private fun chunk(generation: Long = 1, frames: Int = 441, spec: PcmSpec = this.spec) =
    PcmChunk(generation, spec, ByteBuffer.allocate(frames * 2), frames, 0, 0)

  private fun take(writer: Writer, outcomes: Outcomes, limitUs: Long = Long.MAX_VALUE) = VoiceTake(1, file, settings, limitUs, writer, outcomes)

  @Test
  fun noWriterAndNoFileBeforeTheFirstPcm() {
    val writer = Writer()
    take(writer, Outcomes())
    assertEquals(emptyList<String>(), writer.calls)
    assertFalse(file.exists())
  }

  @Test
  fun theFirstPcmOpensTheWriterAndStartsTheTake() {
    val writer = Writer()
    val outcomes = Outcomes()
    val take = take(writer, outcomes)
    take.pcm(chunk(generation = 2))
    assertEquals(emptyList<String>(), writer.calls)
    take.pcm(chunk())
    take.pcm(chunk())
    assertEquals(listOf("open", "append", "append"), writer.calls)
    assertEquals(listOf("started"), outcomes.events)
  }

  @Test
  fun finishDoesNotWaitForTheFileAndReportsOnce() {
    val writer = Writer()
    val outcomes = Outcomes()
    val take = take(writer, outcomes)
    take.pcm(chunk())
    take.finish()
    assertEquals(listOf("started"), outcomes.events)
    writer.done!!(Result.success(10_000L))
    take.cancel()
    take.lost()
    assertEquals(listOf("started", "finished:10000"), outcomes.events)
    assertTrue(file.exists())
  }

  @Test
  fun cancelWhileTheFileClosesRemovesItAfterTheCompletion() {
    val writer = Writer()
    val outcomes = Outcomes()
    val take = take(writer, outcomes)
    take.pcm(chunk())
    take.finish()
    take.cancel()
    assertEquals(listOf("started"), outcomes.events)
    assertTrue(file.exists())
    writer.done!!(Result.success(10_000L))
    assertEquals(listOf("started", "cancelled"), outcomes.events)
    assertFalse(file.exists())
    assertFalse(writer.calls.contains("cancel"))
  }

  @Test
  fun cancelLostAndFailureLeaveNoFileAndOneOutcome() {
    val ends = listOf<(VoiceTake) -> Unit>({ it.cancel() }, { it.lost() }, { it.fail(IllegalStateException("codec")) })
    val expected = listOf("cancelled", "lost", "failed")
    ends.forEachIndexed { index, end ->
      val writer = Writer()
      val outcomes = Outcomes()
      val take = take(writer, outcomes)
      take.pcm(chunk())
      assertTrue(file.exists())
      end(take)
      take.cancel()
      take.lost()
      assertEquals(listOf("started", expected[index]), outcomes.events)
      assertFalse(file.exists())
    }
  }

  @Test
  fun endBeforeTheFirstPcmHasNoWriterCallAndNoFile() {
    val writer = Writer()
    val outcomes = Outcomes()
    take(writer, outcomes).lost()
    assertEquals(listOf("lost"), outcomes.events)
    assertEquals(emptyList<String>(), writer.calls)
    assertFalse(file.exists())
  }

  @Test
  fun aWriterThatIsNotReadyIsOneFailureAndNoFile() {
    val writer = Writer(failOpen = true)
    val outcomes = Outcomes()
    val take = take(writer, outcomes)
    take.pcm(chunk())
    take.pcm(chunk())
    assertEquals(listOf("failed"), outcomes.events)
    assertFalse(file.exists())
  }

  @Test
  fun aPcmOfAnotherFormatFailsBeforeTheWriterOpens() {
    val writer = Writer()
    val outcomes = Outcomes()
    take(writer, outcomes).pcm(chunk(spec = spec.copy(sampleRate = 48_000)))
    assertEquals(listOf("failed"), outcomes.events)
    assertFalse(writer.calls.contains("open"))
  }

  @Test
  fun theLimitAsksForFinishOnce() {
    val writer = Writer()
    val outcomes = Outcomes()
    val take = take(writer, outcomes, limitUs = 15_000)
    take.pcm(chunk(frames = 441))
    take.pcm(chunk(frames = 441))
    take.pcm(chunk(frames = 441))
    assertEquals(listOf("started", "limit"), outcomes.events)
  }
}
