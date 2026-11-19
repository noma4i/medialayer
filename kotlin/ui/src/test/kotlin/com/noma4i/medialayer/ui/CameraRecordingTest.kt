package com.noma4i.medialayer.ui

import android.graphics.SurfaceTexture
import android.media.AudioFormat
import android.media.MediaFormat
import android.media.MediaRecorder
import android.view.Surface
import com.noma4i.medialayer.CaptureEvent
import com.noma4i.medialayer.FailureSource
import com.noma4i.medialayer.NativeFailure
import com.noma4i.medialayer.RecordEnd
import java.io.File
import java.nio.ByteBuffer
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.Executor
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.take
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(MediaLayerTestRunner::class)
class CameraRecordingTest {
  private class FakeTake(val listener: CameraTakeListener) : CameraTaking {
    val calls = CopyOnWriteArrayList<String>()
    override fun begin() = true.also { calls.add("begin") }
    override fun audio(chunk: PcmChunk) { calls.add("audio ${chunk.generation}") }
    override fun audioTimebase(generation: Long, originNanos: Long) { calls.add("timebase $generation $originNanos") }
    override fun confirm() { calls.add("confirm") }
    override fun finish() { calls.add("finish") }
    override fun cancel() { calls.add("cancel") }
  }

  private val log = CopyOnWriteArrayList<String>()
  private val events = CopyOnWriteArrayList<CaptureEvent>()
  private val takes = CopyOnWriteArrayList<FakeTake>()
  private val written = CopyOnWriteArrayList<WrittenTake>()
  private val recording = CameraRecording(Executor { it.run() }, { events.add(it) }, { written.add(it) }) { _, _, _, listener -> FakeTake(listener).also { takes.add(it) } }
  private val surface = Surface(SurfaceTexture(0))
  private val spec = PcmSpec(MediaRecorder.AudioSource.CAMCORDER, 44_100, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
  private val feed = object : CameraEncoderFeed {
    override fun attach(generation: Long, target: Surface) { log.add("attach $generation") }
    override fun detach(generation: Long) { log.add("detach $generation") }
    override fun encoded(generation: Long, ptsUs: Long) { log.add("encoded $generation") }
  }

  private fun settings(audio: Boolean, audioGeneration: Long = 5) =
    CameraTakeSettings(MediaFormat(), if (audio) MediaFormat() else null, VideoTimebase.BOOTTIME, audioGeneration, 1L shl 20, 10_000_000L)

  private fun start(generation: Long, audio: Boolean, stale: () -> Boolean = { false }) =
    recording.start(File("take-$generation.mp4"), settings(audio), feed, { work -> log.add("gate $generation"); work() }, stale, generation) { log.add("finalized $generation") }

  private fun chunk(generation: Long) = PcmChunk(generation, spec, ByteBuffer.allocate(2), 1, 0, 0)

  @Test
  fun a_take_confirms_only_after_its_own_first_pcm_and_starts_after_that() {
    start(1, true)
    takes[0].listener.onBegan(1, surface)
    assertEquals("the surface is attached and the gate waits for sound", listOf("attach 1"), log.toList())
    recording.onPcm(chunk(4))
    assertEquals("the pcm of another generation opens nothing", listOf("begin"), takes[0].calls.toList())
    assertEquals("and gives no flow", emptyList<CaptureEvent>(), events.toList())
    recording.onPcm(chunk(5))
    recording.onPcm(chunk(5))
    assertEquals(listOf("begin", "audio 5", "confirm", "audio 5"), takes[0].calls.toList())
    assertEquals("the gate is asked once", listOf("attach 1", "gate 1"), log.toList())
    assertEquals("the first pcm of the take gives one flow and nothing starts before the take says so", listOf<CaptureEvent>(CaptureEvent.AudioFlow(true)), events.toList())
    takes[0].listener.onStarted(1, 100)
    takes[0].listener.onVideoEncoded(1, 40_100)
    assertEquals(listOf<CaptureEvent>(CaptureEvent.AudioFlow(true), CaptureEvent.RecordStarted), events.toList())
    assertEquals(40L, recording.recordedMs())
    recording.stop()
    takes[0].listener.onFinished(1, File("take-1.mp4"), 40_000, CoreCoverage.covered())
    assertEquals(listOf<CaptureEvent>(CaptureEvent.AudioFlow(true), CaptureEvent.RecordStarted, CaptureEvent.RecordFinished(RecordEnd.COMPLETED)), events.toList())
    assertEquals(false, recording.active)
    assertEquals(listOf("attach 1", "gate 1", "encoded 1", "detach 1", "finalized 1"), log.toList())
  }

  @Test
  fun the_length_and_the_frame_of_a_finished_take_are_the_ones_its_writer_reports() {
    val video = MediaFormat().apply { setInteger(MediaFormat.KEY_WIDTH, 720); setInteger(MediaFormat.KEY_HEIGHT, 1280) }
    recording.start(File("take-1.mp4"), CameraTakeSettings(video, null, VideoTimebase.BOOTTIME, 0, 1L shl 20, 10_000_000L), feed, { work -> work() }, { false }, 1) {}
    takes[0].listener.onBegan(1, surface)
    takes[0].listener.onStarted(1, 0)
    recording.stop()
    takes[0].listener.onFinished(1, File("take-1.mp4"), 2_345_678, CoreCoverage.covered())
    assertEquals(listOf(2_345_678L to com.noma4i.medialayer.ImageSize(720u, 1280u)), written.map { it.durationUs to it.frame })
  }

  @Test
  fun a_take_whose_microphone_was_lost_ends_with_microphone_lost_and_the_others_keep_their_ends() {
    start(1, true)
    takes[0].listener.onBegan(1, surface)
    recording.onPcm(chunk(5))
    takes[0].listener.onStarted(1, 0)
    takes[0].listener.onFinished(1, File("take-1.mp4"), 3_000_000, CoreCoverage.covered(), true)
    assertEquals(CaptureEvent.RecordFinished(RecordEnd.MICROPHONE_LOST), events.last())
    assertEquals(listOf(RecordEnd.MICROPHONE_LOST to 3_000_000L), written.map { it.end to it.durationUs })
    start(2, true)
    takes[1].listener.onBegan(2, surface)
    recording.onPcm(chunk(5))
    takes[1].listener.onStarted(2, 0)
    recording.stop()
    takes[1].listener.onFinished(2, File("take-2.mp4"), 2_000_000, CoreCoverage.covered())
    assertEquals(CaptureEvent.RecordFinished(RecordEnd.COMPLETED), events.last())
    start(3, true)
    takes[2].listener.onBegan(3, surface)
    recording.onPcm(chunk(5))
    takes[2].listener.onStarted(3, 0)
    takes[2].listener.onFinished(3, File("take-3.mp4"), 10_000_000, CoreCoverage.covered())
    assertEquals(CaptureEvent.RecordFinished(RecordEnd.LIMIT_REACHED), events.last())
    assertEquals(listOf(RecordEnd.MICROPHONE_LOST, RecordEnd.COMPLETED, RecordEnd.LIMIT_REACHED), written.map { it.end })
  }

  @Test
  fun a_take_without_sound_confirms_at_once_and_a_limit_finishes_it_by_itself() {
    start(1, false)
    takes[0].listener.onBegan(1, surface)
    assertEquals(listOf("attach 1", "gate 1"), log.take(2))
    takes[0].listener.onStarted(1, 0)
    takes[0].listener.onFinished(1, File("take-1.mp4"), 10_000_000, CoreCoverage.covered())
    assertEquals(CaptureEvent.RecordFinished(RecordEnd.LIMIT_REACHED), events.last())
  }

  @Test
  fun the_timebase_of_the_source_arrives_once_and_reaches_every_new_take() {
    recording.onTimebase(5, 99)
    start(1, true)
    assertEquals(listOf("timebase 5 99", "begin"), takes[0].calls.toList())
    takes[0].listener.onCancelled(1)
    start(2, true)
    assertEquals("the second take gets the stored origin without a second word of the source", listOf("timebase 5 99", "begin"), takes[1].calls.toList())
    recording.onTimebase(6, 7)
    assertEquals("a timebase of another generation is not for this take", 2, takes[1].calls.size)
  }

  @Test
  fun a_take_that_ends_before_it_started_gives_one_failure_and_releases_the_stop() {
    start(1, true)
    takes[0].listener.onBegan(1, surface)
    var stopped = 0
    recording.stop { stopped++ }
    assertEquals(listOf("begin", "cancel"), takes[0].calls.toList())
    takes[0].listener.onCancelled(1)
    takes[0].listener.onCancelled(1)
    takes[0].listener.onFinished(1, File("take-1.mp4"), 1, CoreCoverage.covered())
    assertEquals(listOf<CaptureEvent>(CaptureEvent.RecordFinished(RecordEnd.FAILED)), events.toList())
    assertEquals(1, stopped)
  }

  @Test
  fun a_restored_source_without_a_take_starts_nothing_and_a_stale_take_stays_silent() {
    recording.onPcm(chunk(5))
    assertEquals(emptyList<FakeTake>(), takes.toList())
    var stale = false
    start(1, true) { stale }
    takes[0].listener.onBegan(1, surface)
    stale = true
    recording.onPcm(chunk(5))
    takes[0].listener.onStarted(1, 0)
    takes[0].listener.onFinished(1, File("take-1.mp4"), 1, CoreCoverage.covered())
    assertEquals(emptyList<CaptureEvent>(), events.toList())
    assertEquals("a stale take is detached and never finalized for the app", listOf("attach 1", "detach 1"), log.toList())
  }

  @Test
  fun an_encoder_failure_keeps_its_typed_error_and_ends_the_take_with_one_outcome() = runBlocking {
    start(1, false)
    takes[0].listener.onBegan(1, surface)
    val lost = NativeFailure(FailureSource.RECORDER, "CameraVideoOutput", 3L, "swap failed")
    val failures = async(Dispatchers.Default, start = CoroutineStart.UNDISPATCHED) { Media.failures.take(1).toList() }
    recording.encoderFailed(1, lost)
    assertEquals(listOf(lost), withTimeout(5_000) { failures.await() })
    assertEquals(listOf("begin", "confirm", "cancel"), takes[0].calls.toList())
    takes[0].listener.onCancelled(1)
    takes[0].listener.onFailed(1, lost)
    assertEquals(listOf<CaptureEvent>(CaptureEvent.RecordFinished(RecordEnd.FAILED)), events.toList())
  }

  @Test
  fun inv_capture_79_a_late_encoder_failure_after_the_take_ended_cancels_nothing_and_warns_nothing() {
    val queue = java.util.concurrent.LinkedBlockingQueue<Runnable>()
    val real = CameraRecording(Executor { queue.add(it) }, { events.add(it) }) { generation, file, settings, listener ->
      CameraTake(generation, file, settings, { AutoCloseable {} }, listener)
    }
    val unknown = CameraTakeSettings(MediaFormat(), null, VideoTimebase.UNKNOWN, 5, 1L shl 20, 10_000_000L)
    real.start(File("take-late.mp4"), unknown, feed, { it() }, { false }, 1) {}
    val ended = queue.poll(5, java.util.concurrent.TimeUnit.SECONDS)!!
    val deadline = System.nanoTime() + 5_000_000_000L
    while (Thread.getAllStackTraces().keys.any { it.name == "CameraTake-1" && it.isAlive } && System.nanoTime() < deadline) Thread.sleep(5)
    real.encoderFailed(1, NativeFailure(FailureSource.RECORDER, "CameraVideoOutput", 3L, "swap failed"))
    queue.take().run()
    ended.run()
    queue.toList().also { queue.clear() }.forEach { it.run() }
    val warned = org.robolectric.shadows.ShadowLog.getLogs().filter { it.msg.contains("dead thread") }
    assertEquals("a cancel after the end of the take posts nothing to its dead thread", emptyList<String>(), warned.map { it.msg })
    assertEquals(listOf<CaptureEvent>(CaptureEvent.RecordFinished(RecordEnd.FAILED)), events.toList())
  }

  @Test
  fun the_late_words_of_a_closed_or_replaced_take_change_no_progress_and_no_credit() {
    start(1, false)
    takes[0].listener.onBegan(1, surface)
    takes[0].listener.onStarted(1, 100)
    takes[0].listener.onVideoEncoded(1, 40_100)
    recording.close()
    start(2, false)
    takes[1].listener.onBegan(2, surface)
    takes[1].listener.onStarted(2, 0)
    takes[0].listener.onVideoEncoded(1, 900_000)
    takes[0].listener.onStarted(1, 5)
    takes[0].listener.onFailed(1, NativeFailure(FailureSource.RECORDER, "old", 1L, "late"))
    takes[0].listener.onFinished(1, File("take-1.mp4"), 1, CoreCoverage.covered())
    takes[1].listener.onVideoEncoded(2, 50)
    assertEquals("only the encoded frame of the current take counts", 0L, recording.recordedMs())
    assertEquals(true, recording.active)
    assertEquals(listOf("attach 1", "gate 1", "encoded 1", "detach 1", "attach 2", "gate 2", "encoded 2"), log.toList())
    recording.close()
    start(3, false) { true }
    takes[2].listener.onBegan(3, surface)
    takes[2].listener.onStarted(3, 0)
    takes[2].listener.onVideoEncoded(3, 70_000)
    assertEquals("a stale take returns no credit and gives no progress", 0L, recording.recordedMs())
    assertEquals(listOf("attach 1", "gate 1", "encoded 1", "detach 1", "attach 2", "gate 2", "encoded 2", "detach 2", "attach 3"), log.toList())
    assertEquals(listOf<CaptureEvent>(CaptureEvent.RecordStarted, CaptureEvent.RecordStarted), events.toList())
  }

  @Test
  fun a_take_that_cannot_begin_gives_one_failure_and_no_second_take_runs() {
    val failing = CameraRecording(Executor { it.run() }, { events.add(it) }) { _, _, _, _ -> throw IllegalStateException("no encoder") }
    failing.start(File("take.mp4"), settings(false), feed, { it() }, { false }, 1) {}
    assertEquals(listOf<CaptureEvent>(CaptureEvent.RecordFinished(RecordEnd.FAILED)), events.toList())
    assertEquals(false, failing.active)
  }

  @Test
  fun the_vertex_matrix_turns_the_sensor_upright_and_mirrors_only_when_asked() {
    fun map(matrix: FloatArray) = FloatArray(4).also { android.opengl.Matrix.multiplyMV(it, 0, matrix, 0, floatArrayOf(1f, 0f, 0f, 1f), 0) }.map { Math.round(it) }
    assertEquals(listOf(0, -1, 0, 1), map(uprightVertex(90, false)))
    assertEquals(listOf(1, 0, 0, 1), map(uprightVertex(0, false)))
    assertEquals(listOf(-1, 0, 0, 1), map(uprightVertex(0, true)))
  }
}
