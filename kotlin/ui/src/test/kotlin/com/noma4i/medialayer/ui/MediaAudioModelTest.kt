package com.noma4i.medialayer.ui

import android.app.Application
import androidx.camera.core.ImageAnalysis
import androidx.camera.view.PreviewView
import androidx.lifecycle.LifecycleOwner
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.CameraLens
import com.noma4i.medialayer.CameraPhase
import com.noma4i.medialayer.CameraRefusal
import com.noma4i.medialayer.CaptureEvent
import com.noma4i.medialayer.CaptureKind
import com.noma4i.medialayer.CaptureProfile
import com.noma4i.medialayer.ImageSize
import com.noma4i.medialayer.MediaInfo
import com.noma4i.medialayer.PixelSize
import com.noma4i.medialayer.RecordEnd
import com.noma4i.medialayer.VoiceRefusal
import com.noma4i.medialayer.captureProfile
import java.io.File
import java.util.concurrent.atomic.AtomicLong
import kotlin.random.Random
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeoutOrNull
import org.junit.Test
import org.junit.runner.RunWith

private class SystemFocus : AudioFocusPort {
  private val lock = Any()
  private var holder: FocusKind? = null
  private var taken = false
  private var changes = 0
  private var listener: () -> Unit = {}

  val held: FocusKind? get() = synchronized(lock) { holder }
  val takenBySystem: Boolean get() = synchronized(lock) { taken }
  override val losses: Int get() = synchronized(lock) { changes }

  override fun onChange(action: () -> Unit) {
    synchronized(lock) { listener = action }
  }

  override fun setCommunication(on: Boolean, speaker: Boolean) = Unit

  override fun request(kind: FocusKind): Boolean {
    val granted = synchronized(lock) {
      if (!taken) holder = kind
      !taken
    }
    TestSignal.fire()
    return granted
  }

  override fun abandon() {
    synchronized(lock) { holder = null }
    TestSignal.fire()
  }

  fun systemTakes() = change { holder = null; taken = true; changes++ }

  fun systemGivesBack() = change { taken = false }

  private fun change(effect: () -> Unit) {
    val action = synchronized(lock) {
      effect()
      listener
    }
    action()
    TestSignal.fire()
  }
}

private class ModelBackend(private val refuses: Boolean) : CaptureBackend {
  @Volatile private var sink: (CaptureEvent) -> Unit = {}
  override val photoSize: PixelSize? = PixelSize(3000u, 4000u)

  private fun send(event: CaptureEvent) {
    sink(event)
    TestSignal.fire()
  }

  override fun setSink(sink: (CaptureEvent) -> Unit) {
    this.sink = sink
  }

  override fun setHost(view: PreviewView?, owner: LifecycleOwner?) = Unit

  override fun bind(lens: CameraLens, profile: CaptureProfile) {
    if (refuses) return send(CaptureEvent.Refuse(CameraRefusal.PERMISSION_DENIED))
    send(CaptureEvent.Opened(false, 1f, 4f))
    send(CaptureEvent.Streaming(true))
  }

  override fun unbind() = TestSignal.fire()

  override suspend fun takePhoto(file: File, flash: Boolean, profile: CaptureProfile) = error("no photo in this model")

  override fun beginRecording(file: File, profile: CaptureProfile, lens: CameraLens) {
    file.writeBytes(byteArrayOf(0, 0, 0, 1))
    send(CaptureEvent.RecordStarted)
  }

  override fun endRecording() = send(CaptureEvent.RecordFinished(RecordEnd.COMPLETED))

  override fun recordedMs(): Long = 2_000

  override fun setTorch(on: Boolean) = Unit

  override fun setZoom(ratio: Float) = Unit

  override fun setFrameTap(analyzer: ImageAnalysis.Analyzer?) = Unit

  override fun sensorToView(): android.graphics.Matrix? = null
}

private class ModelMicrophone(private val refuses: Boolean) : VoiceCapture {
  @Volatile private var file: File? = null

  override fun start(file: File, maxDurationMs: Long, onLimit: () -> Unit) {
    if (refuses) throw VoiceRefused(VoiceRefusal.PERMISSION_DENIED)
    this.file = file
  }

  override fun peak(): Float = 0.5f

  override fun stop() {
    file?.writeBytes(byteArrayOf(1, 2, 3))
  }
}

private enum class Step {
  CameraShows, CameraHides, CameraRecords, CameraStops, CameraCancels, CameraDeniedShows, CameraCloses,
  VoiceStarts, VoiceStops, VoiceStartsTwice, VoiceStopsWhileStarting, VoiceDenied,
  PlayerAudible, PlayerSilent, SystemTakesFocus, SystemGivesFocusBack
}

private class World(private val directory: File) {
  val system = SystemFocus()
  val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
  val audio = MediaAudioSession(MediaAudioFocus.Duck, system, releaseDelayMs = 0, scope = scope)
  private val host = Any()
  private val clock = AtomicLong()
  var camera: MediaCamera? = null
  var cameraShown = false
  var voice = recorder(false)
  val audible = HashSet<UInt>()

  fun recorder(refuses: Boolean) = MediaVoiceRecorder(ModelMicrophone(refuses), 60_000, { audio }, { clock.addAndGet(500) }, 5, scope)

  fun makeCamera(refuses: Boolean): MediaCamera {
    val services = CameraServices(
      admitSession = { AutoCloseable {} },
      admitShot = { AutoCloseable {} },
      audio = { audio },
      clipInfo = { MediaInfo(2000UL, ImageSize(720u, 1280u)) },
      uprightVideo = {},
      upright = { file, _ -> file },
      directory = directory
    )
    return MediaCamera(captureProfile(CaptureKind.CIRCLE), CameraLens.FRONT, ModelBackend(refuses), services)
  }

  private val cameraHoldsMicrophone: Boolean
    get() = cameraShown && camera?.state?.value?.phase in setOf(CameraPhase.READY, CameraPhase.RECORDING, CameraPhase.STOPPING)

  val wantsMicrophone: Boolean get() = voice.isRecording || cameraHoldsMicrophone

  fun converged(): Boolean {
    if (cameraShown && camera?.state?.value?.phase == CameraPhase.OPENING) return false
    if (audio.isRecording != wantsMicrophone) return false
    if (system.takenBySystem) return true
    return when {
      wantsMicrophone -> system.held == FocusKind.Recording
      audible.isNotEmpty() -> system.held == FocusKind.Duck
      else -> system.held == null
    }
  }

  fun describe() =
    "camera=${camera?.state?.value?.phase} shown=$cameraShown voice=${voice.isRecording} recordingDemand=${audio.isRecording} " +
      "audible=$audible held=${system.held} taken=${system.takenBySystem}"

  private fun file() = File(directory, "voice-${System.nanoTime()}.m4a")

  suspend fun run(step: Step, random: Random) {
    when (step) {
      Step.CameraShows -> {
        if (camera == null) {
          camera = makeCamera(false)
        }
        camera?.attachPreview(host, null, null)
        cameraShown = true
      }
      Step.CameraHides -> {
        camera?.detachPreview(host)
        cameraShown = false
      }
      Step.CameraRecords -> runCatching { withTimeoutOrNull(2_000) { camera?.startRecording() } }
      Step.CameraStops -> runCatching { withTimeoutOrNull(2_000) { camera?.stopRecording() } }
      Step.CameraCancels -> camera?.cancelRecording()
      Step.CameraDeniedShows -> {
        camera?.close()
        camera = makeCamera(true).also { it.attachPreview(host, null, null) }
        cameraShown = true
      }
      Step.CameraCloses -> {
        camera?.close()
        camera = null
        cameraShown = false
      }
      Step.VoiceStarts -> runCatching { voice.start(file()) }
      Step.VoiceStops -> runCatching { voice.stop() }
      Step.VoiceStartsTwice -> {
        val recorder = voice
        listOf(scope.async { runCatching { recorder.start(file()) } }, scope.async { runCatching { recorder.start(file()) } }).awaitAll()
      }
      Step.VoiceStopsWhileStarting -> {
        val recorder = voice
        val starting = scope.async { runCatching { recorder.start(file()) } }
        runCatching { recorder.stop() }
        starting.await()
      }
      Step.VoiceDenied -> if (!voice.isRecording) {
        runCatching { recorder(true).start(file()) }
      }
      Step.PlayerAudible -> {
        val slot = random.nextInt(1, 4).toUInt()
        audible.add(slot)
        audio.setAudible(slot, true)
      }
      Step.PlayerSilent -> {
        val slot = random.nextInt(1, 4).toUInt()
        audible.remove(slot)
        audio.setAudible(slot, false)
      }
      Step.SystemTakesFocus -> system.systemTakes()
      Step.SystemGivesFocusBack -> system.systemGivesBack()
    }
  }
}

@RunWith(MediaLayerTestRunner::class)
class MediaAudioModelTest {
  private val context = ApplicationProvider.getApplicationContext<Application>()

  @Test
  fun inv_audio_12_anySequenceOfRecordersPlayersAndSystemEventsLeavesTheFocusMatchingTheDemand() {
    val seed = System.getenv("ML_SEED")?.toLongOrNull() ?: Random.nextLong()
    println("repeat: ML_SEED=$seed")
    val random = Random(seed)
    val directory = File(context.cacheDir, "audio-model-$seed").apply { mkdirs() }
    val world = World(directory)
    val trail = ArrayList<Step>()
    repeat(200) {
      val step = Step.entries[random.nextInt(Step.entries.size)]
      trail.add(step)
      runBlocking { world.run(step, random) }
      try {
        TestSignal.await("focus matches the demand after ${trail.takeLast(8)} (ML_SEED=$seed)", 5_000) { world.converged() }
      } catch (failure: AssertionError) {
        throw AssertionError("${failure.message}; ${world.describe()}")
      }
    }
    world.system.systemGivesBack()
    world.camera?.close()
    world.camera = null
    world.cameraShown = false
    runCatching { runBlocking { world.voice.stop() } }
    world.audible.forEach { world.audio.setAudible(it, false) }
    world.audible.clear()
    TestSignal.await("the focus is given back at the end (ML_SEED=$seed)", 5_000) { world.converged() && world.system.held == null }
  }
}
