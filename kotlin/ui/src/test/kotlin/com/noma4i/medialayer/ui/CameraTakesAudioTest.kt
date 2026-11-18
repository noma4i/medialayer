package com.noma4i.medialayer.ui

import android.app.Application
import android.graphics.SurfaceTexture
import android.media.AudioFormat
import android.media.MediaFormat
import android.media.MediaRecorder
import android.view.Surface
import androidx.camera.core.ImageAnalysis
import androidx.camera.view.PreviewView
import androidx.lifecycle.LifecycleOwner
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.CameraLens
import com.noma4i.medialayer.CameraPhase
import com.noma4i.medialayer.CaptureEvent
import com.noma4i.medialayer.CaptureKind
import com.noma4i.medialayer.CaptureProfile
import com.noma4i.medialayer.ImageSize
import com.noma4i.medialayer.MediaInfo
import com.noma4i.medialayer.PixelSize
import com.noma4i.medialayer.captureProfile
import java.io.File
import java.nio.ByteBuffer
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.Executor
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(MediaLayerTestRunner::class)
class CameraTakesAudioTest {
  private class FakeTake(val generation: Long, val listener: CameraTakeListener) : CameraTaking {
    override fun begin() = true
    override fun audio(chunk: PcmChunk) = Unit
    override fun audioTimebase(generation: Long, originNanos: Long) = Unit
    override fun confirm() { TestSignal.fire() }
    @Volatile var finishing = false
    override fun finish() { finishing = true; TestSignal.fire() }
    override fun cancel() = Unit
  }

  private class Microphone : CameraMicrophone {
    val starts = java.util.concurrent.atomic.AtomicLong()
    override fun start(spec: PcmSpec): Long = starts.incrementAndGet().also { TestSignal.fire() }
    override fun stop(generation: Long) = Unit
    override fun close() = Unit
  }

  private val context = ApplicationProvider.getApplicationContext<Application>()
  private val fixture = AudioFixture(context)
  private val seen = CopyOnWriteArrayList<CaptureEvent>()
  private val takes = CopyOnWriteArrayList<FakeTake>()
  private val surface = Surface(SurfaceTexture(0))
  private val spec = PcmSpec(MediaRecorder.AudioSource.CAMCORDER, 44_100, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
  private lateinit var microphone: RecordingAudioListener
  private val source = Microphone()
  private val timer = ManualTimer()

  private val backend = object : CaptureBackend {
    @Volatile var events: (CaptureEvent) -> Unit = {}
    @Volatile var issues: (CaptureAudioIssue) -> Unit = {}
    val recording = CameraRecording(Executor { it.run() }, { seen.add(it); events(it) }) { generation, _, _, listener -> FakeTake(generation, listener).also { takes.add(it); TestSignal.fire() } }
    val audio = CameraAudioHold({ fixture.link }, recording, { issues(it) }, timer) { made -> microphone = made; source }
    private var generation = 0L
    override val photoSize: PixelSize? = null
    override fun setSink(sink: (CaptureEvent) -> Unit) { events = sink }
    override fun setAudioIssueSink(sink: (CaptureAudioIssue) -> Unit) { issues = sink }
    override fun setHost(view: PreviewView?, owner: LifecycleOwner?) = Unit
    override fun bind(lens: CameraLens, profile: CaptureProfile) {
      audio.keep(spec)
      events(CaptureEvent.Opened(false, 1f, 1f))
      events(CaptureEvent.Streaming(true))
    }
    override fun unbind() { recording.close(); audio.keep(null) }
    override suspend fun takePhoto(file: File, flash: Boolean, profile: CaptureProfile) = Unit
    override fun beginRecording(file: File, profile: CaptureProfile, lens: CameraLens) = Unit
    override fun beginRecording(file: File, profile: CaptureProfile, lens: CameraLens, start: (() -> Unit) -> Unit) {
      val settings = CameraTakeSettings(MediaFormat(), MediaFormat(), VideoTimebase.BOOTTIME, checkNotNull(audio.generation), 1L shl 20, 60_000_000L)
      val feed = object : CameraEncoderFeed {
        override fun attach(generation: Long, target: Surface) = Unit
        override fun detach(generation: Long) = Unit
        override fun encoded(generation: Long, ptsUs: Long) = Unit
      }
      recording.start(file, settings, feed, start, { false }, ++generation) {}
    }
    override fun endRecording() = recording.stop()
    override fun recordedMs(): Long = recording.recordedMs()
    override fun setTorch(on: Boolean) = Unit
    override fun setZoom(ratio: Float) = Unit
    override fun setFrameTap(analyzer: ImageAnalysis.Analyzer?) = Unit
    override fun sensorToView(): android.graphics.Matrix? = null
  }

  private val camera = MediaCamera(
    captureProfile(CaptureKind.MOMENT).copy(minTakeMs = 0uL), CameraLens.BACK, backend,
    CameraServices({ AutoCloseable {} }, { AutoCloseable {} }, { fixture.link }, { MediaInfo(2000UL, ImageSize(720u, 1280u)) }, {}, { raw, _ -> raw }, File(context.cacheDir, "takes-${System.nanoTime()}"), { _ -> CoreCoverage.covered() })
  )

  @After fun close() {
    camera.close()
    fixture.close()
  }

  private fun chunk(generation: Long) = PcmChunk(generation, spec, ByteBuffer.allocate(2), 1, 0, 0)

  private fun take(number: Int) = runBlocking {
    val started = async(Dispatchers.Default, start = CoroutineStart.UNDISPATCHED) { camera.startRecording() }
    TestSignal.await("take $number made", 5_000) { takes.size == number }
    val current = takes.last()
    current.listener.onBegan(current.generation, surface)
    microphone.onPcm(chunk(1))
    TestSignal.await("the first pcm of the take confirmed it", 5_000) { seen.count { it == CaptureEvent.AudioFlow(true) } == number }
    current.listener.onStarted(current.generation, 0)
    started.await()
    val stopped = async(Dispatchers.Default, start = CoroutineStart.UNDISPATCHED) { camera.stopRecording() }
    TestSignal.await("the take is asked to finish", 5_000) { current.finishing }
    current.listener.onFinished(current.generation, File("take-$number.mp4"), 1_000_000)
    stopped.await()
  }

  @Test
  fun two_takes_on_one_live_microphone_each_start_with_their_own_first_pcm_and_one_record_started() {
    camera.attachPreview(this, null, null)
    TestSignal.await("ready", 5_000) { camera.state.value.phase == CameraPhase.READY }
    microphone.onLive(1)
    take(1)
    take(2)
    assertEquals("one RecordStarted for each take", 2, seen.count { it == CaptureEvent.RecordStarted })
    seen.withIndex().filter { it.value == CaptureEvent.RecordStarted }.forEach { started ->
      assertEquals("the flow of the take comes before its start", true, seen.subList(0, started.index).any { it == CaptureEvent.AudioFlow(true) })
    }
    microphone.onPcm(chunk(1))
    camera.state.value
    assertEquals("a pcm with no take opens nothing", 2, takes.size)
    assertEquals("a pcm with no take gives no flow", 2, seen.count { it == CaptureEvent.AudioFlow(true) })
  }

  private fun silence(attempt: Long) {
    TestSignal.await("attempt $attempt", 5_000) { source.starts.get() == attempt }
    microphone.onSilenced(attempt, true)
    TestSignal.await("pause after attempt $attempt", 5_000) { timer.armed(MicrophoneRetry.PAUSE_MS) }
    timer.fire(MicrophoneRetry.PAUSE_MS)
  }

  @Test
  fun inv_audio_115_a_camera_take_whose_microphone_is_silenced_twice_starts_on_the_third_attempt_as_one_take() = runBlocking {
    camera.attachPreview(this@CameraTakesAudioTest, null, null)
    TestSignal.await("ready", 5_000) { camera.state.value.phase == CameraPhase.READY }
    val started = async(Dispatchers.Default, start = CoroutineStart.UNDISPATCHED) { camera.startRecording() }
    TestSignal.await("take made", 5_000) { takes.size == 1 }
    val current = takes.last()
    current.listener.onBegan(current.generation, surface)
    silence(1)
    silence(2)
    TestSignal.await("attempt 3", 5_000) { source.starts.get() == 3L }
    microphone.onLive(3)
    microphone.onPcm(chunk(3))
    TestSignal.await("the first pcm of attempt 3 confirmed the take", 5_000) { CaptureEvent.AudioFlow(true) in seen }
    current.listener.onStarted(current.generation, 0)
    kotlinx.coroutines.withTimeout(5_000) { started.await() }
    assertEquals(CameraPhase.RECORDING, camera.state.value.phase)
    assertEquals("one take", 1, takes.size)
    assertEquals(1, seen.count { it == CaptureEvent.RecordStarted })
    assertEquals("the retries tell no silence", false, CaptureEvent.AudioFlow(false) in seen)
    assertEquals(3L, source.starts.get())
  }

  @Test
  fun inv_audio_116_a_camera_take_whose_microphone_is_silenced_on_all_three_attempts_is_microphone_silent() = runBlocking {
    camera.attachPreview(this@CameraTakesAudioTest, null, null)
    TestSignal.await("ready", 5_000) { camera.state.value.phase == CameraPhase.READY }
    val started = async(Dispatchers.Default, start = CoroutineStart.UNDISPATCHED) { runCatching { camera.startRecording() } }
    TestSignal.await("take made", 5_000) { takes.size == 1 }
    val current = takes.last()
    current.listener.onBegan(current.generation, surface)
    silence(1)
    silence(2)
    TestSignal.await("attempt 3", 5_000) { source.starts.get() == 3L }
    microphone.onSilenced(3, true)
    TestSignal.await("the silent take is asked to end", 5_000) { current.finishing }
    current.listener.onCancelled(current.generation)
    val error = kotlinx.coroutines.withTimeout(5_000) { started.await() }.exceptionOrNull() as com.noma4i.medialayer.MediaException.Camera
    assertEquals(com.noma4i.medialayer.CameraRefusal.MICROPHONE_SILENT, error.reason)
    assertEquals("exactly three sources", 3L, source.starts.get())
    assertEquals(false, timer.armed(MicrophoneRetry.PAUSE_MS))
  }
}
