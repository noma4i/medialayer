package com.noma4i.medialayer.ui

import android.content.Context
import android.content.pm.PackageManager
import android.graphics.BitmapFactory
import android.graphics.Matrix
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.view.PreviewView
import androidx.lifecycle.LifecycleOwner
import com.noma4i.medialayer.CameraLens
import com.noma4i.medialayer.CameraPhase
import com.noma4i.medialayer.CameraRefusal
import com.noma4i.medialayer.CameraState
import com.noma4i.medialayer.CaptureAction
import com.noma4i.medialayer.CaptureEvent
import com.noma4i.medialayer.CaptureFileKind
import com.noma4i.medialayer.CaptureProfile
import com.noma4i.medialayer.CaptureRun
import com.noma4i.medialayer.ImageSize
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaInfo
import com.noma4i.medialayer.cameraRefusalText
import com.noma4i.medialayer.captureCost
import com.noma4i.medialayer.captureRun
import com.noma4i.medialayer.captureStep
import com.noma4i.medialayer.captureTickMs
import com.noma4i.medialayer.checkCaptureProfile
import com.noma4i.medialayer.frameGate
import com.noma4i.medialayer.shotCost
import java.io.File
import java.util.UUID
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.Executors
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/** A photo from the camera: an upright JPEG file (the EXIF turn is applied, the mirror is not), its size after the turn and the side it was taken from. */
data class CapturedPhoto(val file: File, val width: Int, val height: Int, val lens: CameraLens)

/** A recorded clip: an mp4 file, its length and its frame as the screen shows it. The file belongs to the app from now on. */
data class CapturedClip(val file: File, val durationMs: Long, val frame: ImageSize?, val lens: CameraLens)

val CameraState.isReady: Boolean get() = phase == CameraPhase.READY

val CameraState.isRecording: Boolean get() = phase == CameraPhase.RECORDING

/**
 * The place of a camera: it takes photos and records video with the profile it was made with. The session runs only while a [MediaCameraPreview] of this camera is on screen
 * and its lifecycle is started, so the app never prepares, resumes or closes the session. [close] ends the camera for good (`rememberMediaCamera` calls it).
 * A command that does not fit the state is ignored and [state] shows the result. A refused permission, a busy camera, a recording that is too short
 * and a refusal of the memory budget are a [MediaException.Camera] with the reason and the text of the core.
 */
class MediaCamera internal constructor(
  private val profile: CaptureProfile,
  lens: CameraLens,
  private val backend: CaptureBackend,
  private val services: CameraServices,
  private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Default),
  private val calls: CallGate = Media.calls
) : AutoCloseable {
  constructor(context: Context, profile: CaptureProfile, lens: CameraLens = CameraLens.BACK) :
    this(profile, lens, CameraXBackend(context.applicationContext), CameraServices.live(context.applicationContext))

  private val queue = Executors.newSingleThreadExecutor { Thread(it, "medialayer-camera").apply { isDaemon = true } }
  private var run: CaptureRun = captureRun(lens)
  private val published = MutableStateFlow(run.state)
  private val hosts = HashSet<Any>()
  private var engaged = false
  private var closed = false
  private var epoch = 0
  private var sessionHold: AutoCloseable? = null
  private var audioHold: MediaRecordingHold? = null
  private var binding: Job? = null
  private var shooting: Job? = null
  private var reading: Job? = null
  private var ticker: Job? = null
  private var recordingFile: File? = null
  private var clipInfo: MediaInfo? = null
  private var ended: Result<CapturedClip>? = null
  private var photoOutcome: Result<CapturedPhoto>? = null
  private var photoWaiter: CompletableDeferred<CapturedPhoto>? = null
  private var startWaiter: CompletableDeferred<Unit>? = null
  private var startFailure: MediaException.Camera? = null
  private var stopWaiter: CompletableDeferred<CapturedClip>? = null
  private val observers = ConcurrentHashMap<String, Observer>()
  private var frameTapOn = false
  private val callListener = CallListener { post(CaptureEvent.CallStarted) }

  private class Observer(val perSecond: Double, val analyzer: ImageAnalysis.Analyzer, @Volatile var last: ULong? = null)

  init {
    checkCaptureProfile(profile)
    backend.setSink { event -> post(event) }
    backend.setAudioIssueSink { issue -> enqueue { microphoneFailed(issue) } }
  }

  /** The state now: readiness, recording and its length, flash, zoom, the side and the last refusal. */
  val state: StateFlow<CameraState> get() = published

  /** The matrix from sensor to view coordinates for the analysis of a frame, null until the preview is laid out. */
  val sensorToView: Matrix? get() = backend.sensorToView()

  suspend fun takePhoto(): CapturedPhoto {
    val waiter = CompletableDeferred<CapturedPhoto>()
    val queued = enqueue {
      if (photoWaiter != null || closed) {
        waiter.completeExceptionally(refusal(CameraRefusal.NOT_READY))
        return@enqueue
      }
      apply(CaptureEvent.Photo)
      if (run.state.shooting) photoWaiter = waiter else waiter.completeExceptionally(refusal(CameraRefusal.NOT_READY))
    }
    if (!queued) throw refusal(CameraRefusal.NOT_READY)
    return waiter.await()
  }

  /** Returns after the hardware started the recording. `NOT_READY` when the camera is not ready or the profile holds no video. */
  suspend fun startRecording() {
    if (profile.audio) {
      val availability = services.audio()?.microphoneAvailability?.value ?: MicrophoneAvailability.AVAILABLE
      if (availability != MicrophoneAvailability.AVAILABLE) throw microphoneRefusal(availability)
    }
    if (calls.inCall) throw refusal(CameraRefusal.IN_CALL)
    val waiter = CompletableDeferred<Unit>()
    val queued = enqueue {
      if (startWaiter != null || closed) {
        waiter.completeExceptionally(refusal(CameraRefusal.NOT_READY))
        return@enqueue
      }
      ended = null
      startFailure = null
      apply(CaptureEvent.Start)
      if (run.starting) startWaiter = waiter else waiter.completeExceptionally(refusal(CameraRefusal.NOT_READY))
    }
    if (!queued) throw refusal(CameraRefusal.NOT_READY)
    waiter.await()
  }

  /**
   * Ends the recording and gives the clip. A recording that is too short or failed, or was thrown away, is a [MediaException.Camera]; a recording that is too short carries its length in `recordedMs`.
   * After the limit ended the recording, gives the outcome of that recording.
   */
  suspend fun stopRecording(): CapturedClip {
    val waiter = CompletableDeferred<CapturedClip>()
    val queued = enqueue {
      val outcome = ended
      if (outcome != null) {
        ended = null
        outcome.fold(waiter::complete, waiter::completeExceptionally)
        return@enqueue
      }
      if (stopWaiter != null || !(run.started || run.starting)) {
        waiter.completeExceptionally(refusal(CameraRefusal.NOT_READY))
        return@enqueue
      }
      stopWaiter = waiter
      apply(CaptureEvent.Stop)
    }
    if (!queued) throw refusal(CameraRefusal.NOT_READY)
    return waiter.await()
  }

  /** Throws the recording away and deletes its file. */
  fun cancelRecording() = post(CaptureEvent.Cancel)

  fun setLens(lens: CameraLens) = post(CaptureEvent.SetLens(lens))

  fun setFlash(on: Boolean) = post(CaptureEvent.SetFlash(on))

  fun zoomBy(factor: Double) = post(CaptureEvent.ZoomBy(factor.toFloat()))

  /**
   * Gives the frames of the preview to [analyzer], at most [maxPerSecond] a second, for the search of a face. The analyzer closes the image.
   * The camera holds no analysis use case while nobody listens.
   */
  fun observeFrames(maxPerSecond: Double, analyzer: ImageAnalysis.Analyzer): AutoCloseable {
    val id = UUID.randomUUID().toString()
    observers[id] = Observer(maxPerSecond, analyzer)
    enqueue { installTap() }
    return AutoCloseable {
      observers.remove(id)
      enqueue { installTap() }
    }
  }

  override fun close() {
    enqueue {
      if (closed) return@enqueue
      hosts.clear()
      reconcile()
      closed = true
      scope.cancel()
    }
    queue.shutdown()
  }

  internal fun attachPreview(host: Any, view: PreviewView?, owner: LifecycleOwner?) {
    enqueue {
      if (closed) return@enqueue
      backend.setHost(view, owner)
      hosts.add(host)
      reconcile()
    }
  }

  internal fun detachPreview(host: Any) {
    enqueue {
      hosts.remove(host)
      reconcile()
    }
  }

  private fun enqueue(block: () -> Unit): Boolean =
    try {
      queue.execute(block)
      true
    } catch (_: java.util.concurrent.RejectedExecutionException) {
      false
    }

  internal suspend fun awaitQueue() {
    val done = CompletableDeferred<Unit>()
    check(enqueue { done.complete(Unit) })
    done.await()
  }

  private fun refusal(reason: CameraRefusal, recordedMs: ULong? = null) = MediaException.Camera(reason, cameraRefusalText(reason), recordedMs)

  private fun microphoneRefusal(value: MicrophoneAvailability) = refusal(value.cameraRefusal())

  private fun post(event: CaptureEvent) {
    if (!queue.isShutdown) enqueue { if (!closed) apply(event) }
  }

  private fun microphoneFailed(issue: CaptureAudioIssue) {
    if (closed || !(run.starting || run.started)) return
    val reason = if (issue == CaptureAudioIssue.PERMISSION_DENIED) MicrophoneAvailability.PERMISSION_DENIED else MicrophoneAvailability.OTHER_APP
    val audio = services.audio()
    audio?.recordingFailed(reason)
    val failure = microphoneRefusal(audio?.microphoneAvailability?.value ?: reason)
    startFailure = failure
    startWaiter?.let { startWaiter = null; it.completeExceptionally(failure) }
    stopWaiter?.let { stopWaiter = null; it.completeExceptionally(failure) }
    apply(CaptureEvent.Refuse(failure.reason))
    ended = Result.failure(failure)
  }

  private fun reconcile() {
    val wanted = hosts.isNotEmpty()
    if (wanted == engaged) return
    engaged = wanted
    apply(if (wanted) CaptureEvent.DemandOn else CaptureEvent.DemandOff)
  }

  private fun apply(event: CaptureEvent) {
    val cancelReading = event == CaptureEvent.Cancel && reading != null
    val step = captureStep(run, event, profile)
    run = step.run
    step.actions.forEach(::perform)
    if (cancelReading) {
      reading?.cancel()
      reading = null
      apply(CaptureEvent.RecordSettled(null))
      return
    }
    published.value = run.state
    onTestChange?.invoke()
    settleWaiters()
    syncTicker()
  }

  private fun settleWaiters() {
    startWaiter?.let { waiter ->
      if (!run.starting) {
        startWaiter = null
        if (run.started) waiter.complete(Unit) else waiter.completeExceptionally(startFailure ?: refusal(CameraRefusal.NOT_READY))
      }
    }
    photoWaiter?.let { waiter ->
      if (!run.state.shooting) {
        photoWaiter = null
        val outcome = photoOutcome ?: Result.failure(refusal(run.state.fault ?: CameraRefusal.NOT_READY))
        photoOutcome = null
        outcome.fold(waiter::complete, waiter::completeExceptionally)
      }
    }
    stopWaiter?.let { waiter ->
      ended?.let {
        stopWaiter = null
        ended = null
        it.fold(waiter::complete, waiter::completeExceptionally)
      }
    }
  }

  private fun syncTicker() {
    if (run.state.phase != CameraPhase.RECORDING) {
      ticker?.cancel()
      ticker = null
      return
    }
    if (ticker != null) return
    val step = captureTickMs().toLong()
    ticker = scope.launch {
      while (isActive) {
        delay(step)
        post(CaptureEvent.Tick(backend.recordedMs().toULong()))
      }
    }
  }

  private fun perform(action: CaptureAction) {
    when (action) {
      is CaptureAction.Bind -> open(action.lens)
      is CaptureAction.Restart -> {
        teardownHardware()
        open(action.lens)
      }
      CaptureAction.Unbind -> {
        epoch += 1
        binding?.cancel()
        shooting?.cancel()
        teardownHardware()
        releaseHolds()
        failWaiters()
      }
      CaptureAction.TakePhoto -> shoot()
      CaptureAction.BeginRecording -> {
        val file = CaptureFiles.name(CaptureFileKind.CLIP, services.directory)
        recordingFile = file
        clipInfo = null
        backend.beginRecording(file, profile, run.state.lens)
        if (!calls.register(callListener)) post(CaptureEvent.CallStarted)
      }
      is CaptureAction.EndRecording -> backend.endRecording()
      CaptureAction.ReadClip -> readClip()
      CaptureAction.Deliver -> deliver()
      CaptureAction.Discard -> discard()
      is CaptureAction.SetTorch -> backend.setTorch(action.on)
      is CaptureAction.SetZoom -> backend.setZoom(action.ratio)
    }
  }

  private fun teardownHardware() {
    backend.unbind()
    calls.unregister(callListener)
    reading?.cancel()
    reading = null
    recordingFile?.delete()
    recordingFile = null
    clipInfo = null
  }

  private fun releaseHolds() {
    sessionHold?.close()
    sessionHold = null
    audioHold?.let {
      audioHold = null
      it.close()
    }
  }

  private fun failWaiters() {
    photoWaiter?.let { photoWaiter = null; it.completeExceptionally(refusal(CameraRefusal.NOT_READY)) }
    stopWaiter?.let { stopWaiter = null; it.completeExceptionally(refusal(CameraRefusal.NOT_READY)) }
  }

  private fun open(lens: CameraLens) {
    epoch += 1
    val current = epoch
    val needsHold = sessionHold == null
    val needsAudio = profile.audio && audioHold == null
    binding?.cancel()
    binding = scope.launch {
      var hold: AutoCloseable? = null
      if (needsHold) {
        try {
          hold = services.admitSession(captureCost(profile).toLong())
        } catch (failure: Exception) {
          if (failure is kotlinx.coroutines.CancellationException) throw failure
          enqueue { if (epoch == current) apply(CaptureEvent.Refuse(CameraRefusal.MEMORY)) }
          return@launch
        }
      }
      var session: MediaRecordingHold? = null
      if (needsAudio) {
        val audio = services.audio()
        val availability = audio?.microphoneAvailability?.value ?: MicrophoneAvailability.AVAILABLE
        if (availability != MicrophoneAvailability.AVAILABLE) {
          hold?.close()
          enqueue { if (epoch == current) apply(CaptureEvent.Refuse(availability.cameraRefusal())) }
          return@launch
        }
        session = audio?.holdRecording()
        if (audio != null && session == null) {
          hold?.close()
          enqueue { if (epoch == current) apply(CaptureEvent.Refuse(audio.microphoneAvailability.value.cameraRefusal())) }
          return@launch
        }
      }
      if (!enqueue {
        if (epoch != current || closed) {
          hold?.close()
          session?.close()
          return@enqueue
        }
        hold?.let { sessionHold = it }
        session?.let { audioHold = it }
        backend.bind(lens, profile)
        installTap()
      }) {
        hold?.close()
        session?.close()
      }
    }
  }

  private fun shoot() {
    val current = epoch
    val flash = run.state.flash
    val lens = run.state.lens
    val bytes = backend.photoSize?.let { shotCost(it.width, it.height) } ?: captureCost(profile)
    shooting = scope.launch {
      fun finish(outcome: Result<CapturedPhoto>) {
        enqueue {
          if (epoch != current || closed) {
            outcome.getOrNull()?.file?.delete()
            return@enqueue
          }
          photoOutcome = outcome
          apply(CaptureEvent.PhotoDone(outcome.isSuccess))
        }
      }
      val hold = try {
        services.admitShot(bytes.toLong())
      } catch (failure: Exception) {
        if (failure is kotlinx.coroutines.CancellationException) throw failure
        return@launch finish(Result.failure(refusal(CameraRefusal.MEMORY)))
      }
      val file = CaptureFiles.name(CaptureFileKind.PHOTO, services.directory)
      val raw = File(file.path + ".raw")
      try {
        backend.takePhoto(raw, flash, profile)
        hold.close()
        val made = services.upright(raw, profile.photoQuality.toDouble() / 100)
        raw.delete()
        check(made.renameTo(file)) { "the photo file cannot be moved" }
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }.also { BitmapFactory.decodeFile(file.path, it) }
        finish(Result.success(CapturedPhoto(file, bounds.outWidth, bounds.outHeight, lens)))
      } catch (failure: Exception) {
        hold.close()
        file.delete()
        if (failure is kotlinx.coroutines.CancellationException) throw failure
        finish(Result.failure(refusal(CameraRefusal.PHOTO_FAILED)))
      } finally {
        raw.delete()
      }
    }
  }

  private fun readClip() {
    val file = recordingFile ?: return
    reading?.cancel()
    reading = scope.launch {
      val info = try {
        services.uprightVideo(file)
        services.clipInfo(file)
      } catch (failure: Exception) {
        if (failure is kotlinx.coroutines.CancellationException) throw failure
        null
      }
      enqueue {
        if (recordingFile != file) return@enqueue
        reading = null
        clipInfo = info
        apply(CaptureEvent.RecordSettled(info?.durationMs))
      }
    }
  }

  private fun deliver() {
    val file = recordingFile
    val info = clipInfo
    val length = info?.durationMs
    if (file == null || info == null || length == null) return discard()
    recordingFile = null
    clipInfo = null
    calls.unregister(callListener)
    ended = Result.success(CapturedClip(file, length.toLong(), info.frame, run.state.lens))
  }

  private fun discard() {
    calls.unregister(callListener)
    reading?.cancel()
    reading = null
    recordingFile?.delete()
    recordingFile = null
    clipInfo = null
    ended = Result.failure(refusal(run.state.fault ?: CameraRefusal.NOT_READY, run.state.refusedMs))
  }

  private fun installTap() {
    if (observers.isEmpty()) {
      if (frameTapOn) backend.setFrameTap(null)
      frameTapOn = false
      return
    }
    frameTapOn = true
    backend.setFrameTap(object : ImageAnalysis.Analyzer {
      override fun analyze(image: ImageProxy) {
        val now = (System.nanoTime() / 1_000_000L).toULong()
        val targets = observers.values.filter { frameGate(it.last, now, it.perSecond) }.onEach { it.last = now }
        if (targets.isEmpty()) return image.close()
        targets.first().analyzer.analyze(image)
      }

      override fun getTargetCoordinateSystem(): Int = observers.values.firstOrNull()?.analyzer?.targetCoordinateSystem ?: ImageAnalysis.COORDINATE_SYSTEM_ORIGINAL

      override fun updateTransform(matrix: Matrix?) {
        observers.values.forEach { it.analyzer.updateTransform(matrix) }
      }
    })
  }

  companion object {
    fun isAvailable(context: Context, lens: CameraLens): Boolean {
      val features = context.packageManager
      return if (lens == CameraLens.FRONT) features.hasSystemFeature(PackageManager.FEATURE_CAMERA_FRONT) else features.hasSystemFeature(PackageManager.FEATURE_CAMERA)
    }
  }
}
