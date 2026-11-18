package com.noma4i.medialayer.ui

import android.content.Context
import android.media.AudioFormat
import android.media.MediaRecorder
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.AudioFocus
import com.noma4i.medialayer.DeviceMemory
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.OpenConfig
import com.noma4i.medialayer.defaultLimits
import com.noma4i.medialayer.defaultPoolSettings
import java.nio.file.Files
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.Executor
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.take
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeoutOrNull
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(MediaLayerTestRunner::class)
class CameraAudioHoldTest {
  private val calls = CopyOnWriteArrayList<String>()
  private var made = 0
  private val listeners = CopyOnWriteArrayList<RecordingAudioListener>()

  private inner class Microphone(val id: Int) : CameraMicrophone {
    override fun start(spec: PcmSpec): Long = 1L.also { calls.add("start $id ${spec.sampleRate}"); TestSignal.fire() }
    override fun stop(generation: Long) { calls.add("stop $id"); TestSignal.fire() }
    override fun close() { calls.add("close $id"); TestSignal.fire() }
  }

  private object Tap : CameraAudioTap {
    override fun onTimebase(generation: Long, originNanos: Long) = Unit
    override fun onPcm(chunk: PcmChunk) = Unit
    // The end of a source is the event the test waits for: availability no longer changes when an input opens or closes (INV-AUDIO-86).
    override fun onSourceEnd(generation: Long) = TestSignal.fire()
  }

  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val core = MediaLayer.open(OpenConfig(
    dir = Files.createTempDirectory("camera-audio-hold").toString(),
    fetcher = TestFetcher(),
    settings = SharedPreferencesSettings(context, "camera-audio-hold"), slots = null,
    device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings(),
    audioFocus = AudioFocus.EXCLUSIVE
  ))
  private val link = AudioLink(core, AudioPorts(RecordingFocusPort(), GrantedPermission))
  private var audio: AudioLink? = link
  private val hold = CameraAudioHold({ audio }, Tap, {}) { listener -> listeners.add(listener); Microphone(++made) }

  @After fun close() {
    hold.close()
    link.close()
    core.close()
  }

  private fun spec(rate: Int) = PcmSpec(MediaRecorder.AudioSource.CAMCORDER, rate, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)

  private fun await(what: String, condition: () -> Boolean) = TestSignal.await(what, 5_000, condition)

  @Test
  fun a_photo_or_a_video_without_sound_opens_no_input_and_the_same_format_keeps_the_one_it_has() {
    hold.keep(null)
    assertEquals(null, hold.generation)
    assertTrue(calls.isEmpty())
    hold.keep(spec(44_100))
    await("the microphone starts") { calls.toList() == listOf("start 1 44100") }
    hold.keep(spec(44_100))
    hold.keep(spec(44_100))
    assertEquals("a rebind with the same format creates nothing", 1, made)
    assertEquals(1L, hold.generation)
  }

  @Test
  fun a_new_format_replaces_the_input_and_audio_turned_off_or_a_photo_closes_it_and_the_next_audio_opens_a_fresh_one() {
    hold.keep(spec(44_100))
    await("first") { calls.toList() == listOf("start 1 44100") }
    hold.keep(spec(48_000))
    await("the old input is closed, the new one starts") { calls.toList() == listOf("start 1 44100", "stop 1", "close 1", "start 2 48000") }
    hold.keep(null)
    await("no sound closes it") { calls.takeLast(2) == listOf("stop 2", "close 2") }
    assertEquals(null, hold.generation)
    hold.keep(spec(48_000))
    await("a later video opens a fresh input") { calls.last() == "start 3 48000" }
  }

  @Test
  fun without_an_audio_link_a_wanted_format_is_a_refusal_and_nothing_stays_open() {
    audio = null
    assertThrows(IllegalStateException::class.java) { hold.keep(spec(44_100)) }
    assertEquals(null, hold.generation)
    assertTrue(calls.isEmpty())
    audio = link
    hold.keep(spec(44_100))
    await("the link came back") { calls.toList() == listOf("start 1 44100") }
  }

  @Test
  fun closing_twice_stops_the_microphone_once_and_a_closed_hold_opens_nothing_again() {
    hold.keep(spec(44_100))
    await("started") { calls.toList() == listOf("start 1 44100") }
    hold.close()
    hold.close()
    await("closed once") { calls.toList() == listOf("start 1 44100", "stop 1", "close 1") }
    assertThrows(IllegalStateException::class.java) { hold.keep(spec(44_100)) }
    assertEquals(null, hold.generation)
    assertEquals(3, calls.size)
  }

  @Test
  fun a_microphone_restarted_after_the_session_memory_closed_ends_quietly_with_no_failure() = runBlocking {
    val memory = CaptureMemory(1L shl 20)
    val quiet = CameraAudioHold({ link }, Tap, {}) { listener -> RecordingAudio(listener, memory::lease, Executor { it.run() }).asMicrophone() }
    val failures = async(Dispatchers.Default) { withTimeoutOrNull(1_500) { Media.failures.take(1).toList() } }
    memory.close()
    quiet.keep(spec(44_100))
    await("the restart met the closed memory and ended") { quiet.generation == null }
    assertEquals("a closed session is no device failure", null, failures.await())
    quiet.close()
  }
}
