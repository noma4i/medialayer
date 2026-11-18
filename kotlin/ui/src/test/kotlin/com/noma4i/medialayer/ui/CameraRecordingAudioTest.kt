package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.SurfaceTexture
import android.media.AudioFormat
import android.media.MediaFormat
import android.media.MediaRecorder
import android.view.Surface
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.AudioFocus
import com.noma4i.medialayer.CaptureEvent
import com.noma4i.medialayer.DeviceMemory
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.OpenConfig
import com.noma4i.medialayer.defaultLimits
import com.noma4i.medialayer.defaultPoolSettings
import java.io.File
import java.nio.ByteBuffer
import java.nio.file.Files
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.Executor
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(MediaLayerTestRunner::class)
class CameraRecordingAudioTest {
  private class FakeTake(val listener: CameraTakeListener) : CameraTaking {
    val calls = CopyOnWriteArrayList<String>()
    override fun begin() = true.also { calls.add("begin") }
    override fun audio(chunk: PcmChunk) { calls.add("audio ${chunk.generation}") }
    override fun audioTimebase(generation: Long, originNanos: Long) { calls.add("timebase $generation $originNanos") }
    override fun confirm() { calls.add("confirm") }
    override fun finish() { calls.add("finish") }
    override fun audioEnded() { calls.add("audioEnded") }
    override fun cancel() { calls.add("cancel") }
  }

  private class Microphone : CameraMicrophone {
    val calls = CopyOnWriteArrayList<String>()
    private var generation = 0L
    override fun start(spec: PcmSpec): Long = (++generation).also { calls.add("start $it"); TestSignal.fire() }
    override fun stop(generation: Long) { calls.add("stop $generation"); TestSignal.fire() }
    override fun close() { calls.add("close"); TestSignal.fire() }
  }

  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val focus = RecordingFocusPort()
  private val core = MediaLayer.open(OpenConfig(
    dir = Files.createTempDirectory("camera-recording-audio").toString(),
    fetcher = TestFetcher(),
    settings = SharedPreferencesSettings(context, "camera-recording-audio"), slots = null,
    device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings(),
    audioFocus = AudioFocus.EXCLUSIVE
  ))
  private val link = AudioLink(core, AudioPorts(focus, GrantedPermission))
  private val microphone = Microphone()
  private val events = CopyOnWriteArrayList<CaptureEvent>()
  private val takes = CopyOnWriteArrayList<FakeTake>()
  private val gates = CopyOnWriteArrayList<String>()
  private val recording = CameraRecording(Executor { it.run() }, { events.add(it) }) { _, _, _, listener -> FakeTake(listener).also { takes.add(it) } }
  private val surface = Surface(SurfaceTexture(0))
  private val spec = PcmSpec(MediaRecorder.AudioSource.CAMCORDER, 44_100, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
  private lateinit var listener: RecordingAudioListener
  private val hold = CameraAudioHold({ link }, recording, {}) { made -> listener = made; microphone }
  private val feed = object : CameraEncoderFeed {
    override fun attach(generation: Long, target: Surface) = Unit
    override fun detach(generation: Long) = Unit
    override fun encoded(generation: Long, ptsUs: Long) = Unit
  }

  @After fun close() {
    hold.close()
    link.close()
    core.close()
  }

  private fun await(what: String, condition: () -> Boolean) = TestSignal.await(what, 5_000, condition)

  private fun chunk(generation: Long) = PcmChunk(generation, spec, ByteBuffer.allocate(2), 1, 0, 0)

  private fun start(generation: Long, audio: Long) =
    recording.start(File("take-$generation.mp4"), CameraTakeSettings(MediaFormat(), MediaFormat(), VideoTimebase.BOOTTIME, audio, 1L shl 20, 10_000_000L), feed, { work -> gates.add("gate $generation"); work() }, { false }, generation) {}

  @Test
  fun a_take_of_the_live_audio_confirms_on_its_own_first_pcm_and_the_next_take_gets_the_stored_timebase() {
    hold.keep(spec)
    await("the microphone started") { microphone.calls == listOf("start 1") }
    listener.onLive(1)
    listener.onTimebase(1, 77)
    start(1, hold.generation!!)
    takes[0].listener.onBegan(1, surface)
    assertEquals("a take confirms only with sound", emptyList<String>(), gates.toList())
    listener.onPcm(chunk(1))
    listener.onPcm(chunk(1))
    assertEquals(listOf("timebase 1 77", "begin", "audio 1", "confirm", "audio 1"), takes[0].calls.toList())
    takes[0].listener.onStarted(1, 0)
    recording.stop()
    takes[0].listener.onFinished(1, File("take-1.mp4"), 1)
    start(2, hold.generation!!)
    takes[1].listener.onBegan(2, surface)
    assertEquals("the second take has the origin of the source without a new word and no confirm before its pcm", listOf("timebase 1 77", "begin"), takes[1].calls.toList())
    listener.onPcm(chunk(1))
    assertEquals(listOf("timebase 1 77", "begin", "audio 1", "confirm"), takes[1].calls.toList())
    assertEquals(listOf("gate 1", "gate 2"), gates.toList())
  }

  @Test
  fun a_call_stops_the_source_and_the_end_of_the_call_neither_feeds_nor_restarts_the_take() {
    hold.keep(spec)
    await("the microphone started") { microphone.calls == listOf("start 1") }
    listener.onLive(1)
    start(1, hold.generation!!)
    takes[0].listener.onBegan(1, surface)
    focus.send(FocusSignal.PhoneCall(true))
    await("the call stops the source") { microphone.calls == listOf("start 1", "stop 1") }
    listener.onPcm(chunk(1))
    focus.send(FocusSignal.PhoneCall(false))
    await("the end of the call restores the source at a new generation") { microphone.calls == listOf("start 1", "stop 1", "start 2") }
    listener.onPcm(chunk(2))
    assertEquals("the stop of the source ends the sound of the take once; neither the late pcm nor the new generation reaches it", listOf("begin", "audioEnded"), takes[0].calls.toList())
    assertEquals(emptyList<String>(), gates.toList())
  }

  @Test
  fun inv_capture_62_closing_the_source_ends_the_sound_of_the_take_without_a_silenced_word_and_only_once() {
    hold.keep(spec)
    await("the microphone started") { microphone.calls == listOf("start 1") }
    listener.onLive(1)
    start(1, hold.generation!!)
    takes[0].listener.onBegan(1, surface)
    hold.close()
    hold.close()
    assertEquals(listOf("begin", "audioEnded"), takes[0].calls.toList())
  }
}
