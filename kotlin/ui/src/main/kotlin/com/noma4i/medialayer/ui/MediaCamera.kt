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
import com.noma4i.medialayer.RecordingKind
import com.noma4i.medialayer.RecordingRefusal
import com.noma4i.medialayer.cameraRefusalText
import com.noma4i.medialayer.audioCoverageRefusal
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
data class CapturedClip(val file: File, val durationMs: Long, val frame: ImageSize?, val lens: CameraLens, val end: RecordEnd)

val CameraState.isReady: Boolean get() = phase == CameraPhase.READY

val CameraState.isRecording: Boolean get() = phase == CameraPhase.RECORDING

/**
 * The place of a camera: it takes photos and records video with the profile it was made with. The session runs only while a [MediaCameraPreview] of this camera is on screen
 * and its lifecycle is started, so the app never prepares, resumes or closes the session. [close] ends the camera for good (`rememberMediaCamera` calls it).
 * A command that does not fit the state is ignored and [state] shows the result. A refused permission, a busy camera, a recording that is too short
 * and a refusal of the memory budget are a [MediaException.Camera] with the reason and the text of the core.
 */
class MediaCamera internal constructor(
  internal val profile: CaptureProfile,
  lens: CameraLens,
  private val backend: CaptureBackend,
  private val services: CameraServices,
  private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Default),
  screen: ScreenAwake = ScreenAwake.shared
) : AutoCloseable {
  constructor(context: Context, profile: CaptureProfile, lens: CameraLens = CameraLens.BACK) :
    this(profile, lens, CameraXBackend(knowing(context)), CameraServices.live(context.applicationContext))

  init { checkCaptureProfile(profile) }
  private val queue = Executors.newSingleThreadExecutor { Thread(it, "medialayer-camera").apply { isDaemon = true } }
  private var run = CaptureRun(
    CameraState(CameraPhase.IDLE, lens, false, false, 1f, 1f, 1f, 0uL, false, null, null, null, null),
    false, false, null, false, false, false, null, false, null, lens
  )
  private val published = MutableStateFlow(run.state)
  private val awake = ScreenAwakeHold(ScreenHoldKind.ALWAYS, screen)
  private val hosts = HashSet<Any>()
  private var engaged = false
  private var closed = false
  private var epoch = 0
  private var sessionHold: AutoCloseable? = null
  private var sessionMemory: CaptureMemory? = null
  private var audioHold: RecordingTicket? = null
  private var audioFailure: RecordingRefusal? = null
  private var retiringAudio: RecordingTicket? = null
  private var recordingStopped: (() -> Unit)? = null
  private var audioRevocation: java.io.Closeable? = null
  private var takeTicket: RecordingTicket? = null
  private var binding: Job? = null
  private var shooting: Job? = null
  private var reading: Job? = null
  private var ticker: Job? = null
  private var recordingFile: File? = null
  private var recordingWritten: WrittenTake? = null
  private var clipInfo: WrittenTake? = null
  private var ended: Result<CapturedClip>? = null
  private var photoOutcome: Result<CapturedPhoto>? = null
  private var photoWaiter: CompletableDeferred<CapturedPhoto>? = null
  private var startWaiter: CompletableDeferred<Unit>? = null
  private var startFailure: MediaException.Camera? = null
  private var stopWaiter: CompletableDeferred<CapturedClip>? = null
  // The delivered clips by take: kept until the app fetches one with `clip(take)` or the camera ends. Read from any thread, so `clip` never waits for the queue.
  private val clips = ConcurrentHashMap<ULong, CapturedClip>()
  private val observers = ConcurrentHashMap<String, Observer>()
  private var frameTapOn = false
  private var frameTapPending = false
  private var torchGeneration = 0L
  private var zoomGeneration = 0L
  private val layerClose = services.audio()?.onClose {
    val done = java.util.concurrent.CountDownLatch(1)
    if (enqueue {
      if (!closed) {
        apply(CaptureEvent.LayerClosed)
        terminate()
      }
      done.countDown()
    }) done.await()
  }

  private class Observer(val perSecond: Double, val analyzer: ImageAnalysis.Analyzer, @Volatile var last: ULong? = null)

  init {
    queue.execute {
      try {
        run = captureRun(lens)
        published.value = run.state
      } catch (error: Exception) {
        run = run.copy(state = run.state.copy(phase = CameraPhase.FAILED))
        published.value = run.state
        Media.recordNativeFailure(com.noma4i.medialayer.FailureSource.CAMERA, error)
      }
    }
    backend.setSink { event -> post(event) }
    backend.setAudioIssueSink { issue -> enqueue { microphoneFailed(issue) } }
    backend.setTakeSink { written -> enqueue { if (!closed) recordingWritten = written } }
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
    return waiter.awaitOwned()
  }

  /** Returns after the hardware started the recording. `NOT_READY` when the camera is not ready or the profile holds no video. */
  suspend fun startRecording() {
    if (profile.audio) ensureAudio()
    val waiter = CompletableDeferred<Unit>()
    val cancelled = java.util.concurrent.atomic.AtomicBoolean()
    val take = java.util.concurrent.atomic.AtomicReference<ULong?>()
    val queued = enqueue {
      if (cancelled.get()) return@enqueue
      if (startWaiter != null || closed) {
        waiter.completeExceptionally(refusal(CameraRefusal.NOT_READY))
        return@enqueue
      }
      ended = null
      startFailure = null
      if (profile.audio && audioHold == null) {
        waiter.completeExceptionally(audioFailure?.let(::microphoneRefusal) ?: refusal(CameraRefusal.NOT_READY))
        return@enqueue
      }
      // The start of a take retries a microphone that ended silent; the take waits for its sound (owner 2026-11-19).
      if (profile.audio) backend.retryMicrophone()
      apply(CaptureEvent.Start)
      take.set(run.state.take)
      if (run.starting) startWaiter = waiter else waiter.completeExceptionally(startFailure ?: refusal(CameraRefusal.NOT_READY))
    }
    if (!queued) throw refusal(CameraRefusal.NOT_READY)
    try { waiter.await() } catch (failure: kotlinx.coroutines.CancellationException) {
      cancelled.set(true)
      enqueue {
        if (startWaiter === waiter) startWaiter = null
        if (take.get() != null && run.state.take == take.get() && (run.starting || run.started)) apply(CaptureEvent.Cancel)
      }
      throw failure
    }
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
        handOver(outcome, waiter)
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
    return waiter.awaitOwned()
  }

  /** The clip of a take once: it stays until this call or the end of the camera, and a later take does not drop it. Never waits for the camera queue. */
  fun clip(take: ULong): CapturedClip? = clips.remove(take)

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
    enqueue { updateTap() }
    return AutoCloseable {
      observers.remove(id)
      enqueue { updateTap() }
    }
  }

  override fun close() {
    enqueue {
      if (closed) return@enqueue
      hosts.clear()
      reconcile()
      layerClose?.close()
      terminate()
    }
  }

  /** The one end of the camera, for the layer close and for [close]: the clips and waiters go, the hardware, the scope and the queue thread stop. Runs on the queue. */
  private fun terminate() {
    closed = true
    ended = null
    clips.values.forEach { deleteMediaFile(it.file) }
    clips.clear()
    failWaiters()
    startWaiter?.let { startWaiter = null; it.completeExceptionally(refusal(CameraRefusal.NOT_READY)) }
    awake.set(false)
    backend.close()
    scope.cancel()
    queue.shutdown()
  }

  /** A caller that gives up cancels the waiter, so the result that comes later is refused by `complete` and its owner removes it. */
  private suspend fun <T> CompletableDeferred<T>.awaitOwned(): T = try { await() } catch (failure: kotlinx.coroutines.CancellationException) {
    cancel()
    throw failure
  }

  private fun handOver(outcome: Result<CapturedClip>, waiter: CompletableDeferred<CapturedClip>) {
    outcome.fold({ clip ->
      if (!clips.values.remove(clip)) waiter.completeExceptionally(refusal(CameraRefusal.NOT_READY))
      else if (!waiter.complete(clip)) run.state.take?.let { clips[it] = clip }
    }, waiter::completeExceptionally)
  }

  internal fun attachPreview(host: Any, view: PreviewView?, owner: LifecycleOwner?) {
    enqueue {
      if (closed) return@enqueue
      backend.setHost(view, owner)
      hosts.add(host)
      awake.set(true)
      reconcile()
    }
  }

  internal fun detachPreview(host: Any) {
    enqueue {
      hosts.remove(host)
      awake.set(hosts.isNotEmpty())
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

  private fun microphoneRefusal(value: RecordingRefusal) = refusal(when (value) {
    RecordingRefusal.IN_CALL -> CameraRefusal.IN_CALL
    RecordingRefusal.INTERRUPTED -> CameraRefusal.INTERRUPTED
    RecordingRefusal.MICROPHONE_SILENT -> CameraRefusal.MICROPHONE_SILENT
    RecordingRefusal.PERMISSION_DENIED -> CameraRefusal.PERMISSION_DENIED
    RecordingRefusal.MICROPHONE_BUSY -> CameraRefusal.MICROPHONE_BUSY
  })

  private suspend fun ensureAudio() {
    val present = CompletableDeferred<Boolean>()
    if (!enqueue { present.complete(audioHold != null) }) throw refusal(CameraRefusal.NOT_READY)
    if (present.await()) return
    val link = services.audio() ?: throw refusal(CameraRefusal.NOT_READY)
    val admission = try { link.admit(RecordingKind.VIDEO) } catch (_: MediaException.Closed) {
      throw refusal(CameraRefusal.NOT_READY)
    }
    val ticket = when (admission) {
      is Admission.Admitted -> admission.ticket
      is Admission.Refused -> throw microphoneRefusal(admission.reason)
    }
    val installed = CompletableDeferred<Boolean>()
    if (!enqueue {
      if (closed || !engaged) { ticket.close(); installed.complete(false) }
      else { installAudio(ticket); installed.complete(true) }
    }) { ticket.close(); throw refusal(CameraRefusal.NOT_READY) }
    if (!installed.await()) throw refusal(CameraRefusal.NOT_READY)
  }

  private fun installAudio(ticket: RecordingTicket) {
    ticket.gate { backend.audioIdle() }
    audioHold?.close()
    audioRevocation?.close()
    audioHold = ticket
    audioFailure = null
    audioRevocation = ticket.onRevoked { reason, done ->
      if (!enqueue {
        if (audioHold !== ticket) { done(); return@enqueue }
        audioHold = null
        audioFailure = reason
        audioRevocation?.close()
        audioRevocation = null
        if (run.starting || run.started) {
          retiringAudio = ticket
          recordingStopped = done
          failMicrophone(microphoneRefusal(reason))
          recordingStopped?.let { pending -> recordingStopped = null; backend.awaitRecordingStopped(pending) }
        } else { ticket.close(); done() }
      }) done()
    }
  }

  private fun post(event: CaptureEvent) {
    if (!queue.isShutdown) enqueue { if (!closed) apply(event) }
  }

  private fun microphoneFailed(issue: CaptureAudioIssue) {
    if (closed) return
    when (issue) {
      CaptureAudioIssue.SILENCED -> { apply(CaptureEvent.AudioFlow(false)); backend.audioLost() }
      CaptureAudioIssue.ACTIVE -> apply(CaptureEvent.AudioFlow(true))
      CaptureAudioIssue.PERMISSION_DENIED -> if (run.starting || run.started) failMicrophone(microphoneRefusal(RecordingRefusal.PERMISSION_DENIED))
      CaptureAudioIssue.SOURCE_ERROR, CaptureAudioIssue.ENCODER_ERROR -> if (run.starting || run.started) failMicrophone(refusal(CameraRefusal.RECORD_FAILED))
    }
  }

  private fun failMicrophone(failure: MediaException.Camera) {
    startFailure = failure
    apply(CaptureEvent.Revoked(failure.reason))
    backend.audioLost()
  }

  private fun reconcile() {
    val wanted = hosts.isNotEmpty()
    if (wanted == engaged) return
    engaged = wanted
    apply(if (wanted) CaptureEvent.DemandOn else CaptureEvent.DemandOff)
  }

  private fun apply(event: CaptureEvent) {
    val cancelReading = if (event == CaptureEvent.Cancel) reading else null
    val previous = run
    val step = captureStep(run, event, profile)
    run = step.run
    try { step.actions.forEach { perform(it, previous.state) } } catch (failure: Exception) {
      run = previous
      Media.recordNativeFailure(com.noma4i.medialayer.FailureSource.CAMERA, failure)
    }
    syncTake()
    if (cancelReading != null) {
      cancelReading.invokeOnCompletion {
        enqueue {
          if (reading === cancelReading) {
            reading = null
            apply(CaptureEvent.RecordSettled(null))
          }
        }
      }
      cancelReading.cancel()
      return
    }
    published.value = run.state
    if (frameTapPending && !run.starting && !run.started) { frameTapPending = false; installTap() }
    onTestChange?.invoke()
    settleWaiters()
    syncTicker()
  }

  // The core hears every take of the camera: a voice is busy during it and takes the microphone of the idle preview after it.
  private fun syncTake() {
    val recording = run.starting || run.started
    val ticket = audioHold
    if (recording && takeTicket == null && ticket != null) { takeTicket = ticket; ticket.taking(true) }
    else if (!recording) takeTicket?.let { takeTicket = null; it.taking(false) }
  }

  private fun settleWaiters() {
    startWaiter?.let { waiter ->
      if (run.state.phase == CameraPhase.RECORDING || (!run.starting && !run.started)) {
        startWaiter = null
        if (run.state.phase == CameraPhase.RECORDING) waiter.complete(Unit) else waiter.completeExceptionally(startFailure ?: refusal(run.state.fault ?: CameraRefusal.NOT_READY))
      }
    }
    photoWaiter?.let { waiter ->
      if (!run.state.shooting) {
        photoWaiter = null
        val outcome = photoOutcome ?: Result.failure(refusal(run.state.fault ?: CameraRefusal.NOT_READY))
        photoOutcome = null
        // A caller that gave up owns no result: its photo file goes with it.
        outcome.fold({ if (!waiter.complete(it)) deleteMediaFile(it.file) }, waiter::completeExceptionally)
      }
    }
    stopWaiter?.let { waiter ->
      ended?.let {
        stopWaiter = null
        ended = null
        handOver(it, waiter)
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

  private fun perform(action: CaptureAction, before: CameraState) {
    when (action) {
      is CaptureAction.Bind -> open(action.lens)
      is CaptureAction.Restart -> {
        if (run.state.outcome != null) {
          ended = Result.failure(refusal(CameraRefusal.INTERRUPTED))
          stopWaiter?.let { stopWaiter = null; it.completeExceptionally(refusal(CameraRefusal.INTERRUPTED)) }
        }
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
        recordingWritten = null
        clipInfo = null
        val ticket = audioHold
        backend.beginRecording(file, profile, run.state.lens) { start ->
          val refused = if (profile.audio) checkNotNull(ticket).start(start) else { start(); null }
          if (refused != null) enqueue { failMicrophone(microphoneRefusal(refused)) }
        }
      }
      is CaptureAction.EndRecording -> {
        val stopped = recordingStopped
        recordingStopped = null
        if (stopped == null) backend.endRecording() else backend.endRecording(stopped)
      }
      CaptureAction.ReadClip -> readClip()
      CaptureAction.Deliver -> deliver()
      CaptureAction.Discard -> discard()
      is CaptureAction.SetTorch -> {
        val generation = ++torchGeneration
        backend.setTorch(action.on) { failure -> refused(failure) { if (generation == torchGeneration) run = run.copy(state = run.state.copy(flash = before.flash)) } }
      }
      is CaptureAction.SetZoom -> {
        val generation = ++zoomGeneration
        backend.setZoom(action.ratio) { failure -> refused(failure) { if (generation == zoomGeneration) run = run.copy(state = run.state.copy(zoom = before.zoom)) } }
      }
    }
  }

  private fun refused(failure: Throwable?, rollback: () -> Unit) {
    if (failure == null) return
    enqueue {
      if (!closed) {
        rollback()
        published.value = run.state
        onTestChange?.invoke()
      }
      Media.recordNativeFailure(com.noma4i.medialayer.FailureSource.CAMERA, failure)
    }
  }

  private fun teardownHardware() {
    backend.unbind()
    reading?.cancel()
    reading = null
    recordingFile?.let(::deleteMediaFile)
    recordingFile = null
    recordingWritten = null
    clipInfo = null
  }

  private fun releaseHolds() {
    val hold = sessionHold
    val memory = sessionMemory
    sessionHold = null
    sessionMemory = null
    // The audio input, the microphone and the take of the session are closed by the backend first; the memory and its hold go after them.
    if (memory != null && hold != null) backend.unbound { memory.close { hold.close() } } else hold?.close()
    audioRevocation?.close()
    audioRevocation = null
    retiringAudio?.close()
    retiringAudio = null
    audioHold?.let {
      audioHold = null
      it.close()
    }
  }

  private fun failWaiters() {
    photoWaiter?.let { photoWaiter = null; it.completeExceptionally(refusal(CameraRefusal.NOT_READY)) }
    stopWaiter?.let { stopWaiter = null; it.completeExceptionally(refusal(CameraRefusal.INTERRUPTED)) }
  }

  private fun open(lens: CameraLens) {
    epoch += 1
    val current = epoch
    val needsHold = sessionHold == null
    val needsAudio = profile.audio && audioHold == null
    binding?.cancel()
    binding = scope.launch {
      var hold: AutoCloseable? = null
      var session: RecordingTicket? = null
      try {
        if (needsHold) {
          try {
            hold = services.admitSession(captureCost(profile).toLong())
          } catch (failure: Exception) {
            if (failure is kotlinx.coroutines.CancellationException) throw failure
            MediaTrace.emit("camera") { "session refused cost=${captureCost(profile)} failure=$failure" }
            val refusal = reservationRefusal(failure)
            enqueue { if (epoch == current) apply(CaptureEvent.Refuse(refusal)) }
            return@launch
          }
        }
        if (needsAudio) {
          val audio = services.audio()
          if (audio == null) {
            enqueue { if (epoch == current) apply(CaptureEvent.Refuse(CameraRefusal.NOT_READY)) }
            return@launch
          }
          val admission = try { audio.admit(RecordingKind.VIDEO) } catch (_: MediaException.Closed) {
            enqueue { if (epoch == current) apply(CaptureEvent.Refuse(CameraRefusal.NOT_READY)) }
            return@launch
          }
          when (admission) {
            is Admission.Admitted -> session = admission.ticket
            is Admission.Refused -> {
              enqueue { if (epoch == current) apply(CaptureEvent.Refuse(microphoneRefusal(admission.reason).reason)) }
              return@launch
            }
          }
        }
        val admittedHold = hold
        val admittedSession = session
        if (enqueue {
          if (epoch != current || closed) {
            try { admittedHold?.close() } finally { admittedSession?.close() }
            return@enqueue
          }
          admittedHold?.let {
            sessionHold = it
            sessionMemory = CaptureMemory(captureCost(profile).toLong()).also(backend::useMemory)
          }
          admittedSession?.let { installAudio(it) }
          backend.bind(lens, profile)
          installTap()
        }) {
          hold = null
          session = null
        }
      } finally {
        try { hold?.close() } finally { session?.close() }
      }
    }
  }

  /** A closed MediaLayer is not a lack of memory: it is not ready, a full budget is memory, any other failure is reported and the camera is not ready. */
  private fun reservationRefusal(failure: Exception): CameraRefusal = when (failure) {
    is MediaException.Closed -> CameraRefusal.NOT_READY
    is MediaException.Overloaded -> CameraRefusal.MEMORY
    else -> { Media.recordNativeFailure(com.noma4i.medialayer.FailureSource.CAMERA, failure); CameraRefusal.NOT_READY }
  }

  private fun shoot() {
    val current = epoch
    val flash = run.state.flash
    val lens = run.state.lens
    val bytes = backend.photoSize?.let { shotCost(it.width, it.height) } ?: captureCost(profile)
    shooting = scope.launch {
      fun finish(outcome: Result<CapturedPhoto>) {
        if (!enqueue {
          if (epoch != current || closed) {
            outcome.getOrNull()?.file?.let(::deleteMediaFile)
            return@enqueue
          }
          photoOutcome = outcome
          apply(CaptureEvent.PhotoDone(outcome.isSuccess))
        }) outcome.getOrNull()?.file?.let(::deleteMediaFile)
      }
      val hold = try {
        services.admitShot(bytes.toLong())
      } catch (failure: Exception) {
        if (failure is kotlinx.coroutines.CancellationException) throw failure
        MediaTrace.emit("camera") { "shot refused cost=$bytes failure=$failure" }
        return@launch finish(Result.failure(refusal(reservationRefusal(failure))))
      }
      val file = try { CaptureFiles.name(CaptureFileKind.PHOTO, services.directory) } catch (failure: Exception) {
        hold.close()
        Media.recordNativeFailure(com.noma4i.medialayer.FailureSource.FILE, failure)
        return@launch finish(Result.failure(refusal(CameraRefusal.PHOTO_FAILED)))
      }
      val raw = File(file.path + ".raw")
      try {
        backend.takePhoto(raw, flash, profile)
        hold.close()
        val made = services.upright(raw, profile.photoQuality.toDouble() / 100)
        deleteMediaFile(raw)
        check(made.renameTo(file)) { "the photo file cannot be moved" }
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }.also { BitmapFactory.decodeFile(file.path, it) }
        finish(Result.success(CapturedPhoto(file, bounds.outWidth, bounds.outHeight, lens)))
      } catch (failure: Exception) {
        hold.close()
        deleteMediaFile(file)
        if (failure is kotlinx.coroutines.CancellationException) throw failure
        Media.recordNativeFailure(com.noma4i.medialayer.FailureSource.CAMERA, failure)
        finish(Result.failure(refusal(CameraRefusal.PHOTO_FAILED)))
      } finally {
        deleteMediaFile(raw)
      }
    }
  }

  /** The clip is judged from what its writer knew at the write point: the verdict of its sound, its length and its frame. The file is never opened here. */
  private fun readClip() {
    val file = recordingFile ?: return
    reading?.cancel()
    val silent = takeRefusal()
    val info = if (silent == null) recordingWritten else null
    reading = scope.launch {
      enqueue {
        if (recordingFile != file) return@enqueue
        if (reading?.isCancelled == true) return@enqueue
        reading = null
        clipInfo = info
        silent?.let { apply(CaptureEvent.Revoked(it)) }
        apply(CaptureEvent.RecordSettled(info?.let { (it.durationUs / 1000).toULong() }))
      }
    }
  }

  /** The refusal of the core for the sound the take wrote, judged at its write point: no file is read. A take with sound whose verdict never arrived is refused as broken. */
  private fun takeRefusal(): CameraRefusal? =
    if (!profile.audio) null else recordingWritten.let { if (it == null) CameraRefusal.AUDIO_TRACK_BROKEN else audioCoverageRefusal(it.coverage.verdict) }

  private fun deliver() {
    val file = recordingFile
    val info = clipInfo
    val take = run.state.take
    if (file == null || info == null || take == null || closed) return discard()
    recordingFile = null
    recordingWritten = null
    clipInfo = null
    val clip = CapturedClip(file, info.durationUs / 1000, info.frame, run.takeLens, info.end)
    clips[take] = clip
    ended = Result.success(clip)
  }

  private fun discard() {
    retiringAudio?.close()
    retiringAudio = null
    reading?.cancel()
    reading = null
    recordingFile?.let(::deleteMediaFile)
    recordingFile = null
    recordingWritten = null
    clipInfo = null
    ended = Result.failure(refusal(run.state.fault ?: CameraRefusal.NOT_READY, run.state.refusedMs))
  }

  private fun updateTap() {
    if (run.starting || run.started) frameTapPending = true else installTap()
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
        if (!enqueue {
          val now = (System.nanoTime() / 1_000_000L).toULong()
          val targets = observers.values.filter { frameGate(it.last, now, it.perSecond) }.onEach { it.last = now }
          if (targets.isEmpty()) image.close() else {
            val remaining = java.util.concurrent.atomic.AtomicInteger(targets.size)
            targets.forEach { target ->
              val closed = java.util.concurrent.atomic.AtomicBoolean()
              val shared = object : ImageProxy by image {
                override fun close() { if (closed.compareAndSet(false, true) && remaining.decrementAndGet() == 0) image.close() }
              }
              try { target.analyzer.analyze(shared) } catch (error: Exception) { shared.close(); Media.recordFailure(error) }
            }
          }
        }) image.close()
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
