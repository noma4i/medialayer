package com.noma4i.medialayer.ui

import android.app.Application
import android.media.AudioFormat
import android.media.MediaRecorder
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.AudioFocus
import com.noma4i.medialayer.DeviceMemory
import com.noma4i.medialayer.FailureSource
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MicrophoneAvailability
import com.noma4i.medialayer.NativeFailure
import com.noma4i.medialayer.OpenConfig
import com.noma4i.medialayer.VoiceEncoderSettings
import com.noma4i.medialayer.VoiceOutcome
import com.noma4i.medialayer.VoicePhase
import com.noma4i.medialayer.VoiceRefusal
import com.noma4i.medialayer.defaultLimits
import com.noma4i.medialayer.defaultPoolSettings
import com.noma4i.medialayer.voiceEncoderSettings
import com.noma4i.medialayer.voiceLevel
import com.noma4i.medialayer.voiceRefusalText
import java.io.File
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.file.Files
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

private class Ticks(@Volatile var now: Long = 1_000) {
  fun read(): ULong = now.toULong()
}

/** The first fake boundary: the microphone of the owner, one generation per start. */
private class Microphone : CameraMicrophone {
  val calls = SignallingList<String>()
  @Volatile var listener: RecordingAudioListener? = null
  private var generation = 0L
  override fun start(spec: PcmSpec): Long = (++generation).also { calls.add("start $it") }
  override fun stop(generation: Long) { calls.add("stop $generation") }
  override fun close() { calls.add("close") }
}

/** The second fake boundary: the file writer, it keeps the file of a take on `open` and ends a take at once. */
private class Writer(private val notReady: Throwable? = null) : VoiceFileWriter {
  val calls = SignallingList<String>()
  @Volatile var settings: VoiceEncoderSettings? = null
  override fun prepare(settings: VoiceEncoderSettings, ready: (Result<Unit>) -> Unit, failed: (Throwable) -> Unit) {
    calls.add("prepare")
    this.settings = settings
    ready(notReady?.let { Result.failure(it) } ?: Result.success(Unit))
  }
  override fun open(file: File) { calls.add("open"); file.writeBytes(byteArrayOf(1, 2, 3)) }
  override fun append(chunk: PcmChunk) { calls.add("append ${chunk.generation}") }
  override fun finish(done: (Result<Long>) -> Unit) { calls.add("finish"); done(Result.success(1L)) }
  override fun cancel() { calls.add("cancel") }
}

@RunWith(MediaLayerTestRunner::class)
class MediaVoiceRecorderTest {
  private val context = ApplicationProvider.getApplicationContext<Application>()
  private val scope = CoroutineScope(Dispatchers.Default)
  private val focus = RecordingFocusPort()
  private val permission = TogglePermission()
  private val core = MediaLayer.open(OpenConfig(
    dir = Files.createTempDirectory("voice-recorder").toString(), fetcher = TestFetcher(),
    settings = SharedPreferencesSettings(context, "voice-recorder"), slots = null,
    device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings(), audioFocus = AudioFocus.EXCLUSIVE
  ))
  private val link = AudioLink(core, AudioPorts(focus, permission))
  private val availability = CopyOnWriteArrayList<MicrophoneAvailability>()
  private val follow = link.onAvailability { availability.add(it); TestSignal.fire() }
  private val failures = SignallingList<NativeFailure>()
  private val watch = scope.launch(start = CoroutineStart.UNDISPATCHED) { Media.failures.collect { failures.add(it) } }
  private val microphone = Microphone()
  private val ticks = Ticks()
  private val spec = PcmSpec(MediaRecorder.AudioSource.MIC, 44_100, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)

  @After fun close() {
    watch.cancel()
    follow.close()
    link.close()
    core.close()
  }

  private fun file(): File = File.createTempFile("voice", ".m4a").apply { delete(); deleteOnExit() }

  private fun recorder(writer: Writer = Writer(), maxMs: Long = 60_000, minTakeMs: ULong = 0uL, screen: ScreenSwitch = ScreenSwitch(), clock: () -> ULong = ticks::read) =
    MediaVoiceRecorder(SystemVoiceCapture(context, { link }, { writer }, { made -> microphone.listener = made; microphone }), maxMs.toULong(), { link }, clock, 5, scope, minTakeMs, screen.awake())

  private fun chunk(generation: Long, sample: Int = 0, frames: Int = 1): PcmChunk {
    val data = ByteBuffer.allocate(frames * 2).order(ByteOrder.LITTLE_ENDIAN)
    repeat(frames) { data.putShort(it * 2, sample.toShort()) }
    return PcmChunk(generation, spec, data, frames, 0, 0)
  }

  private fun await(what: String, condition: () -> Boolean) = TestSignal.await(what, 5_000, condition)

  /** Starts a take and answers its first live PCM: the start returns only after that. */
  private suspend fun record(recorder: MediaVoiceRecorder, target: File, sample: Int = 0, frames: Int = 1) {
    val start = scope.async { recorder.start(target) }
    await("the owner starts the microphone") { microphone.calls.contains("start 1") }
    microphone.listener!!.onLive(1)
    microphone.listener!!.onPcm(chunk(1, sample, frames))
    withTimeout(5_000) { start.await() }
  }

  private fun refusal(work: suspend () -> Unit) = assertThrows(MediaException.Voice::class.java) { runBlocking { work() } }

  @Test
  fun inv_voice_01_no_file_and_no_recording_before_the_first_live_pcm_and_both_after_it() = runBlocking<Unit> {
    val writer = Writer()
    val recorder = recorder(writer)
    val target = file()
    val start = scope.async { recorder.start(target) }
    await("the owner starts the microphone") { microphone.calls.contains("start 1") }

    assertEquals(VoicePhase.IDLE, recorder.state.value.phase)
    assertFalse(target.exists())
    await("no sound before the first PCM") { availability.lastOrNull() == MicrophoneAvailability.NO_SOUND }

    microphone.listener!!.onLive(1)
    microphone.listener!!.onPcm(chunk(1))
    withTimeout(5_000) { start.await() }
    await("the microphone is heard") { availability.lastOrNull() == MicrophoneAvailability.AVAILABLE }

    assertEquals(VoicePhase.RECORDING, recorder.state.value.phase)
    assertTrue(target.isFile)
    assertEquals(voiceEncoderSettings(), writer.settings)
    ticks.now += 1_000
    recorder.stop()
  }

  @Test
  fun inv_voice_04_stop_gives_the_file_the_length_and_one_closed_owner() = runBlocking<Unit> {
    val recorder = recorder()
    val target = file()
    record(recorder, target)

    ticks.now += 2_500
    val recording = recorder.stop()

    assertEquals(target, recording.file)
    assertEquals(2_500L, recording.durationMs)
    assertTrue(recording.file.isFile)
    assertFalse(recorder.isRecording)
    await("the owner closes with the take") { microphone.calls.toList() == listOf("start 1", "stop 1", "close") }
    assertEquals(VoiceOutcome.Recorded(target.absolutePath, 2_500uL), recorder.state.value.outcome)
    assertTrue(failures.isEmpty())
  }

  @Test
  fun inv_api_62_a_running_voice_take_keeps_the_screen_on() = runBlocking<Unit> {
    val screen = ScreenSwitch()
    val recorder = recorder(screen = screen)
    assertFalse(screen.on)

    record(recorder, file())
    await("the screen stays on while the take runs") { screen.on }
    ticks.now += 1_000
    recorder.stop()

    await("the screen may sleep after the take") { !screen.on }
  }

  @Test
  fun inv_audio_69_voice_state_publishes_recorded_time_and_the_levels_follow_the_peak() = runBlocking<Unit> {
    val recorder = recorder()
    val level = scope.async(start = CoroutineStart.UNDISPATCHED) { recorder.levels.first { it == voiceLevel(0.5f) } }
    record(recorder, file(), sample = 16_384)

    ticks.now += 1_200
    val state = withTimeout(5_000) { recorder.state.first { it.recordedMs == 1_200uL } }
    withTimeout(5_000) { level.await() }
    recorder.stop()

    assertEquals(VoicePhase.RECORDING, state.phase)
    assertEquals(VoicePhase.IDLE to 0uL, recorder.state.value.phase to recorder.state.value.recordedMs)
  }

  @Test
  fun inv_audio_70_a_short_voice_take_is_refused_and_deleted() = runBlocking<Unit> {
    val recorder = recorder(minTakeMs = 1_000uL)
    val target = file()
    record(recorder, target)

    ticks.now += 400
    val error = refusal { recorder.stop() }

    assertEquals(VoiceRefusal.TOO_SHORT, error.reason)
    assertEquals(VoiceOutcome.Refused(VoiceRefusal.TOO_SHORT), recorder.state.value.outcome)
    assertFalse(target.exists())
  }

  @Test
  fun a_cancel_while_the_start_waits_leaves_no_file_no_session_and_no_owner() = runBlocking<Unit> {
    val recorder = recorder()
    val target = file()
    val start = scope.async(start = CoroutineStart.UNDISPATCHED) { recorder.start(target) }
    await("the owner starts the microphone") { microphone.calls.contains("start 1") }

    start.cancelAndJoin()

    await("the owner is closed") { microphone.calls.toList() == listOf("start 1", "stop 1", "close") }
    assertTrue(start.isCancelled)
    assertFalse(target.exists())
    assertFalse(link.isRecording)
    assertFalse(recorder.isRecording)
    assertTrue(failures.isEmpty())
  }

  @Test
  fun a_denied_permission_and_a_busy_microphone_are_typed_refusals_with_no_failure() {
    val writer = Writer()
    val recorder = recorder(writer)
    permission.change(false)
    val denied = refusal { recorder.start(file()) }
    permission.change(true)
    focus.answer = com.noma4i.medialayer.FocusAnswer.FAILED
    val busy = refusal { recorder.start(file()) }

    assertEquals(voiceRefusalText(VoiceRefusal.PERMISSION_DENIED), denied.detail)
    assertEquals(voiceRefusalText(VoiceRefusal.MICROPHONE_BUSY), busy.detail)
    assertEquals(VoiceOutcome.Refused(VoiceRefusal.MICROPHONE_BUSY), recorder.state.value.outcome)
    assertTrue(failures.isEmpty())
    assertTrue(microphone.calls.isEmpty() && writer.calls.isEmpty())
    assertFalse(link.isRecording)
  }

  @Test
  fun a_call_while_the_start_waits_is_in_call_and_frees_the_ticket_and_the_owner() = runBlocking<Unit> {
    val recorder = recorder()
    val waiting = scope.async { runCatching { recorder.start(file()) } }
    await("the owner starts the microphone") { microphone.calls.contains("start 1") }
    val call = link.call(MediaCallKind.Unmanaged)
    val error = withTimeout(5_000) { waiting.await() }.exceptionOrNull() as MediaException.Voice
    call.close()

    assertEquals(VoiceRefusal.IN_CALL, error.reason)
    assertEquals(VoiceOutcome.Refused(VoiceRefusal.IN_CALL), recorder.state.value.outcome)
    assertFalse(link.isRecording)
    await("the owner is closed") { microphone.calls.last() == "close" }
  }

  @Test
  fun a_loss_while_recording_ends_the_take_once_without_a_file_and_the_first_healthy_pcm_closes_the_owner() = runBlocking<Unit> {
    val recorder = recorder()
    val target = file()
    record(recorder, target)

    microphone.listener!!.onSilenced(1, true)
    val ended = withTimeout(5_000) { recorder.state.first { it.outcome != null } }
    await("the core restores the source") { microphone.calls.toList().take(3) == listOf("start 1", "stop 1", "start 2") }
    microphone.listener!!.onPcm(chunk(1))
    microphone.listener!!.onPcm(chunk(2, 16_384))
    await("the first healthy PCM closes the owner") { microphone.calls.toList() == listOf("start 1", "stop 1", "start 2", "stop 2", "close") }

    assertEquals(VoiceOutcome.Refused(VoiceRefusal.MICROPHONE_SILENT), ended.outcome)
    assertFalse(target.exists())
    assertEquals("no resume", VoicePhase.IDLE, recorder.state.value.phase)
    assertEquals(1, recorder.state.value.take.toInt())
    assertTrue(failures.isEmpty())
  }

  @Test
  fun an_early_limit_is_the_result_of_start_and_the_take_ends_once() = runBlocking<Unit> {
    val steps = java.util.concurrent.atomic.AtomicLong(1_000)
    val recorder = recorder(maxMs = 1, clock = { steps.getAndAdd(10).toULong() })
    val target = file()
    record(recorder, target, frames = 100)

    val ended = withTimeout(5_000) { recorder.state.first { it.outcome != null } }
    val recording = recorder.stop()

    assertTrue(ended.outcome is VoiceOutcome.Recorded)
    assertEquals(target, recording.file)
    assertEquals(1, ended.take.toInt())
    assertEquals("the owner closed once", 1, microphone.calls.count { it == "close" })
  }

  @Test
  fun a_codec_that_is_not_ready_is_one_failure_with_no_file_and_no_microphone() {
    val recorder = recorder(Writer(notReady = IllegalStateException("no codec")))
    val target = file()

    assertThrows(MediaException.Unexpected::class.java) { runBlocking { recorder.start(target) } }

    await("the failure reaches the app") { failures.size == 1 }
    assertEquals("no codec", failures.single().message)
    assertEquals(FailureSource.RECORDER, failures.single().source)
    assertEquals(VoicePhase.IDLE, recorder.state.value.phase)
    assertTrue(failures.single().let { recorder.state.value.outcome == VoiceOutcome.Failed(it) })
    assertTrue(microphone.calls.isEmpty())
    assertFalse(target.exists())
    assertFalse(link.isRecording)
  }

  @Test
  fun a_native_error_of_the_source_while_recording_is_one_failure_of_the_recorder() = runBlocking<Unit> {
    val recorder = recorder()
    val target = file()
    record(recorder, target)
    val native = NativeFailure(FailureSource.RECORDER, "android.media.AudioRecord", 7L, "dead")

    microphone.listener!!.onFailed(1, native)
    val ended = withTimeout(5_000) { recorder.state.first { it.outcome != null } }
    await("the failure reaches the app") { failures.size == 1 }

    assertEquals(VoiceOutcome.Failed(native), ended.outcome)
    assertEquals(listOf(native), failures.toList())
    assertFalse(target.exists())
    assertFalse(link.isRecording)
  }

  @Test
  fun a_released_core_frees_the_waiting_start_with_its_owner() = runBlocking<Unit> {
    val recorder = recorder()
    val waiting = scope.async { runCatching { recorder.start(file()) } }
    await("the owner starts the microphone") { microphone.calls.contains("start 1") }

    link.close()

    val error = withTimeout(5_000) { waiting.await() }.exceptionOrNull()
    assertTrue(error is MediaException)
    await("the owner is closed") { microphone.calls.last() == "close" }
  }
}
