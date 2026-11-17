package com.noma4i.medialayer.ui

import android.content.Context
import android.media.AudioFormat
import android.media.MediaRecorder
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.AudioFocus
import com.noma4i.medialayer.DeviceMemory
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MicrophoneAvailability
import com.noma4i.medialayer.NativeFailure
import com.noma4i.medialayer.OpenConfig
import com.noma4i.medialayer.VoiceEncoderSettings
import com.noma4i.medialayer.defaultLimits
import com.noma4i.medialayer.defaultPoolSettings
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.file.Files
import java.util.concurrent.CopyOnWriteArrayList
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(MediaLayerTestRunner::class)
class VoiceInputTest {
  private class Microphone : CameraMicrophone {
    val calls = CopyOnWriteArrayList<String>()
    private var generation = 0L
    override fun start(spec: PcmSpec): Long = (++generation).also { calls.add("start $it"); TestSignal.fire() }
    override fun stop(generation: Long) { calls.add("stop $generation"); TestSignal.fire() }
    override fun close() { calls.add("close"); TestSignal.fire() }
  }

  private class Writer : VoiceFileWriter {
    val calls = CopyOnWriteArrayList<String>()
    var ready: ((Result<Unit>) -> Unit)? = null
    var failed: ((Throwable) -> Unit)? = null
    var done: ((Result<Long>) -> Unit)? = null
    override fun prepare(settings: VoiceEncoderSettings, ready: (Result<Unit>) -> Unit, failed: (Throwable) -> Unit) { calls.add("prepare"); this.ready = ready; this.failed = failed }
    override fun open(file: File) { calls.add("open"); file.writeText("m4a") }
    override fun append(chunk: PcmChunk) { calls.add("append ${chunk.generation}") }
    override fun finish(done: (Result<Long>) -> Unit) { calls.add("finish"); this.done = done }
    override fun cancel() { calls.add("cancel") }
  }

  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val focus = RecordingFocusPort()
  private val core = MediaLayer.open(OpenConfig(
    dir = Files.createTempDirectory("voice-input").toString(),
    fetcher = TestFetcher(),
    settings = SharedPreferencesSettings(context, "voice-input"), slots = null,
    device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings(),
    audioFocus = AudioFocus.EXCLUSIVE
  ))
  private val link = AudioLink(core, AudioPorts(focus, GrantedPermission))
  private val availability = CopyOnWriteArrayList<MicrophoneAvailability>()
  private val follow = link.onAvailability { availability.add(it); TestSignal.fire() }
  private val microphone = Microphone()
  private val writer = Writer()
  private val settings = VoiceEncoderSettings(44_100u, 1u, 128_000u)
  private val spec = PcmSpec(MediaRecorder.AudioSource.MIC, 44_100, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
  private val file = File.createTempFile("voice-input", ".m4a").also { it.delete() }
  private val events = CopyOnWriteArrayList<String>()
  private lateinit var microphoneListener: RecordingAudioListener

  private val take = object : VoiceTakeListener {
    override fun onStarted() { events += "started"; TestSignal.fire() }
    override fun onLimit() { events += "limit" }
    override fun onFinished(file: File, durationUs: Long) { events += "finished"; TestSignal.fire() }
    override fun onFailed(failure: NativeFailure) { events += "failed:${failure.message}"; TestSignal.fire() }
    override fun onCancelled() { events += "cancelled"; TestSignal.fire() }
    override fun onLost() { events += "lost"; TestSignal.fire() }
  }

  @After fun close() {
    follow.close()
    link.close()
    core.close()
  }

  private fun input() = VoiceInput(link, spec, settings, Long.MAX_VALUE, file, writer, take) { made -> microphoneListener = made; microphone }

  private fun await(what: String, condition: () -> Boolean) = TestSignal.await(what, 5_000, condition)

  private fun chunk(generation: Long, sample: Int = 0) = PcmChunk(generation, spec, ByteBuffer.allocate(2).order(ByteOrder.LITTLE_ENDIAN).putShort(0, sample.toShort()), 1, 0, 0)

  private fun started(input: VoiceInput) {
    input.start()
    writer.ready!!(Result.success(Unit))
    await("the owner starts the microphone") { microphone.calls == listOf("start 1") }
  }

  @Test
  fun the_microphone_waits_for_the_prepared_codec_and_the_file_waits_for_the_first_live_pcm() {
    val input = input()
    input.start()
    assertEquals("the codec is prepared before any source", listOf("prepare"), writer.calls.toList())
    assertTrue(microphone.calls.isEmpty())
    writer.ready!!(Result.success(Unit))
    await("restore") { microphone.calls == listOf("start 1") }
    assertFalse("no file before the first PCM", file.exists())
    microphoneListener.onLive(1)
    microphoneListener.onPcm(chunk(1, 16_384))
    await("started") { events == listOf("started") }
    assertEquals(listOf("prepare", "open", "append 1"), writer.calls.toList())
    assertTrue(file.exists())
    assertEquals(0.5f, input.peak(), 0f)
    assertEquals("the peak is read once", 0f, input.peak(), 0f)
    input.finish()
    writer.done!!(Result.success(10L))
    await("finished") { events == listOf("started", "finished") }
    await("the owner is closed with the take") { microphone.calls == listOf("start 1", "stop 1", "close") }
  }

  @Test
  fun a_lost_take_ends_once_without_a_file_and_the_owner_closes_on_the_next_healthy_pcm_of_a_later_generation() {
    val input = input()
    started(input)
    microphoneListener.onLive(1)
    microphoneListener.onPcm(chunk(1))
    await("started") { events == listOf("started") }
    microphoneListener.onSilenced(1, true)
    await("lost, the restore is the core's") { events == listOf("started", "lost") && microphone.calls == listOf("start 1", "stop 1", "start 2") }
    assertFalse("a lost take leaves no file", file.exists())
    microphoneListener.onPcm(chunk(1))
    assertEquals("an old generation reaches nothing", listOf("prepare", "open", "append 1", "cancel"), writer.calls.toList())
    microphoneListener.onPcm(chunk(2, 16_384))
    assertEquals("a PCM after the end feeds no peak", 0f, input.peak(), 0f)
    await("the next healthy PCM closes the owner") { microphone.calls == listOf("start 1", "stop 1", "start 2", "stop 2", "close") }
    assertEquals("no new take", listOf("started", "lost"), events.toList())
    assertEquals(listOf("prepare", "open", "append 1", "cancel"), writer.calls.toList())
  }

  @Test
  fun a_loss_while_waiting_for_the_first_pcm_is_told_once_with_no_file_and_the_next_healthy_pcm_closes_the_owner() {
    val input = input()
    started(input)
    microphoneListener.onLive(1)
    microphoneListener.onSilenced(1, true)
    await("lost, the restore is the core's") { events == listOf("lost") && microphone.calls == listOf("start 1", "stop 1", "start 2") }
    microphoneListener.onSilenced(1, true)
    microphoneListener.onPcm(chunk(2, 16_384))
    await("the next healthy PCM closes the owner") { microphone.calls == listOf("start 1", "stop 1", "start 2", "stop 2", "close") }
    assertEquals("no take, no peak", 0f, input.peak(), 0f)
    assertEquals(listOf("lost"), events.toList())
    assertEquals(listOf("prepare", "cancel"), writer.calls.toList())
    assertFalse(file.exists())
  }

  @Test
  fun a_cancel_while_the_codec_is_preparing_frees_it_and_a_late_ready_starts_no_source() {
    val input = input()
    input.start()
    input.cancel()
    writer.ready!!(Result.success(Unit))
    assertEquals(listOf("cancelled"), events.toList())
    assertEquals(listOf("prepare", "cancel"), writer.calls.toList())
    assertTrue(microphone.calls.isEmpty())
  }

  @Test
  fun a_cancel_before_the_first_pcm_frees_the_codec_and_the_owner() {
    val input = input()
    started(input)
    input.cancel()
    await("closed") { microphone.calls == listOf("start 1", "stop 1", "close") }
    assertEquals(listOf("cancelled"), events.toList())
    assertEquals(listOf("prepare", "cancel"), writer.calls.toList())
    assertFalse(file.exists())
  }

  @Test
  fun a_codec_that_is_not_ready_is_one_typed_failure_with_no_source_and_no_file() {
    val input = input()
    input.start()
    writer.ready!!(Result.failure(IllegalStateException("no codec")))
    assertEquals(listOf("failed:no codec"), events.toList())
    assertTrue(microphone.calls.isEmpty())
    assertFalse(file.exists())
  }

  @Test
  fun a_writer_error_after_ready_and_a_source_error_end_the_take_once() {
    val input = input()
    started(input)
    writer.failed!!(IllegalStateException("codec died"))
    microphoneListener.onFailed(1, NativeFailure(com.noma4i.medialayer.FailureSource.RECORDER, "android.media.AudioRecord", 7L, "dead"))
    await("the owner is closed") { microphone.calls.contains("close") }
    assertEquals("one event, one owner", listOf("failed:codec died"), events.toList())
    assertEquals(listOf("prepare", "cancel"), writer.calls.toList())
  }
}
