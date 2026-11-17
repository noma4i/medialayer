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
  private val events = ArrayList<String>()
  private val writer = Writer()

  private class Writer(val failOpen: Boolean = false, val failAppend: Boolean = false) : VoiceFileWriter {
    val calls = ArrayList<String>()
    var done: ((Result<Long>) -> Unit)? = null
    override fun prepare(settings: VoiceEncoderSettings, ready: (Result<Unit>) -> Unit, failed: (Throwable) -> Unit) = Unit
    override fun open(file: File) { calls += "open"; if (failOpen) error("writer is not ready"); file.writeText("m4a") }
    override fun append(chunk: PcmChunk) { calls += "append"; if (failAppend) error("no free input") }
    override fun finish(done: (Result<Long>) -> Unit) { calls += "finish"; this.done = done }
    override fun cancel() { calls += "cancel" }
  }

  private val listener = object : VoiceTakeListener {
    override fun onStarted() { events += "started" }
    override fun onLimit() { events += "limit" }
    override fun onFinished(file: File, durationUs: Long) { events += "finished:$durationUs" }
    override fun onFailed(failure: NativeFailure) { events += "failed" }
    override fun onCancelled() { events += "cancelled" }
    override fun onLost() { events += "lost" }
  }

  private fun chunk(generation: Long = 1, spec: PcmSpec = this.spec) = PcmChunk(generation, spec, ByteBuffer.allocate(882), 441, 0, 0)

  private fun take(writer: Writer = this.writer, limitUs: Long = Long.MAX_VALUE) = VoiceTake(1, file, settings, limitUs, writer, listener)

  private val ends = listOf<Pair<String, (VoiceTake) -> Unit>>("cancelled" to { it.cancel() }, "lost" to { it.lost() }, "failed" to { it.fail(IllegalStateException("codec")) })

  @Test
  fun theFirstPcmOpensTheWriterAndNothingExistsBefore() {
    val take = take()
    assertEquals(emptyList<String>(), writer.calls)
    assertFalse(file.exists())
    take.pcm(chunk(generation = 2))
    take.pcm(chunk())
    take.pcm(chunk())
    assertEquals(listOf("open", "append", "append"), writer.calls)
    assertEquals(listOf("started"), events)
  }

  @Test
  fun finishReportsOnceWhenTheFileCloses() {
    val take = take()
    take.pcm(chunk())
    take.finish()
    take.finish()
    assertEquals(listOf("started"), events)
    writer.done!!(Result.success(10_000L))
    writer.done!!(Result.success(20_000L))
    take.cancel()
    take.lost()
    assertEquals(listOf("started", "finished:10000"), events)
    assertEquals(1, writer.calls.count { it == "finish" })
    assertTrue(file.exists())
  }

  @Test
  fun anEndDuringTheCloseRemovesTheFileAfterTheCompletion() = ends.forEach { (name, end) ->
    events.clear()
    val writer = Writer().also { it.calls.clear() }
    val take = take(writer)
    take.pcm(chunk())
    take.finish()
    end(take)
    take.cancel()
    assertEquals(listOf("started"), events)
    assertTrue(file.exists())
    writer.done!!(Result.success(10_000L))
    assertEquals(listOf("started", name), events)
    assertFalse(file.exists())
    assertFalse(writer.calls.contains("cancel"))
  }

  @Test
  fun anEndWhileRecordingLeavesNoFileAndOneOutcomeAndNoResume() = ends.forEach { (name, end) ->
    events.clear()
    val writer = Writer()
    val take = take(writer)
    take.pcm(chunk())
    end(take)
    take.pcm(chunk())
    take.finish()
    take.lost()
    assertEquals(listOf("started", name), events)
    assertEquals(listOf("open", "append", "cancel"), writer.calls)
    assertFalse(file.exists())
  }

  @Test
  fun anEndBeforeTheFirstPcmHasNoWriterCallAndNoFile() = (ends.map { it.second } + { it: VoiceTake -> it.finish() }).forEach { end ->
    events.clear()
    val writer = Writer()
    val take = take(writer)
    end(take)
    take.pcm(chunk())
    assertEquals(1, events.size)
    assertEquals(listOf("cancel"), writer.calls)
    assertFalse(file.exists())
  }

  @Test
  fun aWriterThatIsNotReadyAndAPcmOfAnotherFormatAreOneFailureAndNoFile() {
    listOf(Writer(failOpen = true) to listOf("open", "cancel"), Writer(failAppend = true) to listOf("open", "append", "cancel")).forEach { (broken, calls) ->
      events.clear()
      take(broken).apply { pcm(chunk()); pcm(chunk()) }
      assertEquals(listOf("failed"), events)
      assertEquals(calls, broken.calls)
      assertFalse(file.exists())
    }
    events.clear()
    take().pcm(chunk(spec = spec.copy(sampleRate = 48_000)))
    assertEquals(listOf("failed"), events)
    assertEquals(listOf("cancel"), writer.calls)
  }

  @Test
  fun theLimitAsksForFinishOnce() {
    val take = take(limitUs = 15_000)
    repeat(3) { take.pcm(chunk()) }
    assertEquals(listOf("started", "limit"), events)
  }
}
