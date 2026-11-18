package com.noma4i.medialayer.ui

import android.content.Context
import android.media.AudioFormat
import android.media.MediaRecorder
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.AudioFocus
import com.noma4i.medialayer.DeviceMemory
import com.noma4i.medialayer.FailureSource
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MicrophoneAvailability
import com.noma4i.medialayer.NativeFailure
import com.noma4i.medialayer.OpenConfig
import com.noma4i.medialayer.defaultLimits
import com.noma4i.medialayer.defaultPoolSettings
import java.nio.ByteBuffer
import java.nio.file.Files
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.take
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(MediaLayerTestRunner::class)
class CameraAudioInputTest {
  private class Microphone : CameraMicrophone {
    val calls = CopyOnWriteArrayList<String>()
    private var generation = 0L
    override fun start(spec: PcmSpec): Long = (++generation).also { calls.add("start $it"); TestSignal.fire() }
    override fun stop(generation: Long) { calls.add("stop $generation"); TestSignal.fire() }
    override fun close() { calls.add("close"); TestSignal.fire() }
  }

  private class Tap : CameraAudioTap {
    val seen = CopyOnWriteArrayList<String>()
    override fun onTimebase(generation: Long, originNanos: Long) { seen.add("timebase $generation"); TestSignal.fire() }
    override fun onPcm(chunk: PcmChunk) { seen.add("pcm ${chunk.generation}"); TestSignal.fire() }
  }

  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val focus = RecordingFocusPort()
  private val core = MediaLayer.open(OpenConfig(
    dir = Files.createTempDirectory("camera-audio").toString(),
    fetcher = TestFetcher(),
    settings = SharedPreferencesSettings(context, "camera-audio"), slots = null,
    device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings(),
    audioFocus = AudioFocus.EXCLUSIVE
  ))
  private val link = AudioLink(core, AudioPorts(focus, GrantedPermission))
  private val availability = CopyOnWriteArrayList<MicrophoneAvailability>()
  private val follow = link.onAvailability { availability.add(it); TestSignal.fire() }
  private val microphone = Microphone()
  private val tap = Tap()
  private val issues = CopyOnWriteArrayList<CaptureAudioIssue>()
  private val spec = PcmSpec(MediaRecorder.AudioSource.CAMCORDER, 44_100, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
  private lateinit var listener: RecordingAudioListener

  @After fun close() {
    follow.close()
    link.close()
    core.close()
  }

  private val timer = ManualTimer()

  private fun open() = CameraAudioInput(link, spec, tap, { issues.add(it) }, timer = timer) { made -> listener = made; microphone }

  private fun await(what: String, condition: () -> Boolean) = TestSignal.await(what, 5_000, condition)

  private fun chunk(generation: Long) = PcmChunk(generation, spec, ByteBuffer.allocate(2), 1, 0, 0)

  @Test
  fun inv_audio_86_the_input_is_available_before_its_first_pcm_and_no_sound_only_after_the_microphone_is_lost() {
    val input = open()
    await("restore") { microphone.calls == listOf("start 1") }
    Thread.sleep(200)
    assertEquals("an input not heard yet does not blink no sound", listOf(MicrophoneAvailability.AVAILABLE), availability.toList())
    assertEquals(1L, input.audioGeneration)
    listener.onLive(1)
    await("heard") { availability.last() == MicrophoneAvailability.AVAILABLE }
    listener.onSilenced(1, true)
    await("silenced restarts the input at a new generation") { microphone.calls == listOf("start 1", "stop 1", "start 2") && availability.last() == MicrophoneAvailability.NO_SOUND }
    listener.onLive(1)
    listener.onSilenced(1, false)
    assertEquals("the late word of the old generation changes nothing", listOf(CaptureAudioIssue.ACTIVE, CaptureAudioIssue.SILENCED), issues.toList())
    assertEquals(MicrophoneAvailability.NO_SOUND, availability.last())
    input.close()
  }

  @Test
  fun an_external_call_stops_the_input_and_its_end_restores_it_without_a_take() {
    val input = open()
    await("restore") { microphone.calls == listOf("start 1") }
    listener.onLive(1)
    await("heard") { availability.last() == MicrophoneAvailability.AVAILABLE }
    focus.send(FocusSignal.PhoneCall(true))
    await("a call stops it") { microphone.calls == listOf("start 1", "stop 1") && availability.last() == MicrophoneAvailability.EXTERNAL_CALL }
    assertEquals(null, input.audioGeneration)
    focus.send(FocusSignal.PhoneCall(false))
    await("the end of the call restores it and waits for sound") { microphone.calls == listOf("start 1", "stop 1", "start 2") && availability.last() == MicrophoneAvailability.AVAILABLE }
    listener.onPcm(chunk(2))
    assertEquals("the live generation passes its pcm and a restore starts no take", listOf("pcm 2"), tap.seen.toList())
    input.close()
  }

  @Test
  fun an_own_call_is_not_an_external_call_and_its_end_starts_no_take() {
    val input = open()
    await("restore") { microphone.calls == listOf("start 1") }
    listener.onLive(1)
    await("heard") { availability.last() == MicrophoneAvailability.AVAILABLE }
    val call = link.call(MediaCallKind.Unmanaged)
    await("an own call stops it") { microphone.calls == listOf("start 1", "stop 1") && availability.last() == MicrophoneAvailability.OWN_CALL }
    call.close()
    await("the end of the call restores it") { microphone.calls == listOf("start 1", "stop 1", "start 2") && availability.last() == MicrophoneAvailability.AVAILABLE }
    assertTrue("the end of a call starts no take", tap.seen.isEmpty())
    input.close()
  }

  @Test
  fun pcm_and_timebase_reach_the_take_only_from_the_live_generation_and_nothing_after_stop() {
    val input = open()
    await("restore") { microphone.calls == listOf("start 1") }
    listener.onTimebase(1, 5)
    listener.onPcm(chunk(1))
    listener.onPcm(chunk(2))
    listener.onTimebase(2, 5)
    assertEquals(listOf("timebase 1", "pcm 1"), tap.seen.toList())
    input.close()
    listener.onPcm(chunk(1))
    listener.onTimebase(1, 6)
    assertEquals("a closed generation delivers nothing", listOf("timebase 1", "pcm 1"), tap.seen.toList())
  }

  @Test
  fun a_failed_microphone_retries_twice_then_reports_its_typed_failure_once_and_waits_for_a_cause_instead_of_looping() = runBlocking {
    val input = open()
    await("restore") { microphone.calls == listOf("start 1") }
    val dead = NativeFailure(FailureSource.RECORDER, "android.media.AudioRecord", 7L, "dead")
    val failures = async(Dispatchers.Default, start = CoroutineStart.UNDISPATCHED) { Media.failures.take(1).toList() }
    for (attempt in 1L..2L) {
      listener.onFailed(attempt, dead)
      await("pause after attempt $attempt") { timer.armed(MicrophoneRetry.PAUSE_MS) }
      assertEquals("a retried failure is told to nobody", 1L, input.audioGeneration)
      timer.fire(MicrophoneRetry.PAUSE_MS)
      await("attempt ${attempt + 1}") { microphone.calls.last() == "start ${attempt + 1}" }
    }
    assertTrue(issues.isEmpty())
    listener.onFailed(3, dead)
    assertEquals(listOf(dead), withTimeout(5_000) { failures.await() })
    assertEquals(listOf(CaptureAudioIssue.SOURCE_ERROR), issues.toList())
    assertEquals(null, input.audioGeneration)
    listener.onFailed(3, NativeFailure(FailureSource.RECORDER, "android.media.AudioRecord", 7L, "again"))
    assertEquals("one failure, one outcome", listOf(CaptureAudioIssue.SOURCE_ERROR), issues.toList())
    assertEquals(listOf("start 1", "start 2", "start 3"), microphone.calls.toList())
    input.close()
  }

  @Test
  fun inv_audio_114_the_pcm_of_a_retried_attempt_reaches_the_take_under_the_generation_of_the_first_and_a_stop_cancels_the_pause() {
    val input = open()
    await("restore") { microphone.calls == listOf("start 1") }
    listener.onSilenced(1, true)
    await("pause") { timer.armed(MicrophoneRetry.PAUSE_MS) }
    assertEquals(1L, input.audioGeneration)
    timer.fire(MicrophoneRetry.PAUSE_MS)
    await("attempt 2") { microphone.calls == listOf("start 1", "stop 1", "start 2") }
    listener.onLive(2)
    listener.onPcm(chunk(2))
    assertEquals(listOf("pcm 1"), tap.seen.toList())
    assertEquals(listOf(CaptureAudioIssue.ACTIVE), issues.toList())
    assertFalse(MicrophoneAvailability.NO_SOUND in availability)
    listener.onSilenced(2, true)
    await("a heard source is not retried") { issues.toList() == listOf(CaptureAudioIssue.ACTIVE, CaptureAudioIssue.SILENCED) }
    input.close()
  }

  @Test
  fun closing_stops_the_microphone_once_and_a_late_restore_starts_nothing() {
    val input = open()
    await("restore") { microphone.calls == listOf("start 1") }
    input.close()
    input.close()
    await("closed") { microphone.calls == listOf("start 1", "stop 1", "close") }
    listener.onLive(1)
    focus.send(FocusSignal.PhoneCall(true))
    focus.send(FocusSignal.PhoneCall(false))
    assertEquals(listOf("start 1", "stop 1", "close"), microphone.calls.toList())
  }
}
