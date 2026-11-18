package com.noma4i.medialayer.ui

import android.content.Context
import android.media.AudioFormat
import android.media.MediaRecorder
import android.os.SystemClock
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.VoiceRefusal
import com.noma4i.medialayer.VoiceState
import com.noma4i.medialayer.VoicePhase
import com.noma4i.medialayer.VoiceOutcome
import com.noma4i.medialayer.NativeFailure
import com.noma4i.medialayer.FailureSource
import com.noma4i.medialayer.RecordingKind
import com.noma4i.medialayer.RecordingRefusal
import com.noma4i.medialayer.voiceEncoderSettings
import com.noma4i.medialayer.voiceLevel
import com.noma4i.medialayer.voiceLimitMs
import com.noma4i.medialayer.voiceRefusalText
import com.noma4i.medialayer.voiceTooShort
import java.io.File
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.channelFlow
import kotlinx.coroutines.flow.emitAll
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/** A finished voice recording: an m4a file (AAC, mono) and its length. */
data class VoiceRecording(val file: File, val durationMs: Long)

internal class VoiceRefused(val reason: VoiceRefusal) : Exception()

internal class VoiceNativeFailure(val failure: NativeFailure) : Exception(failure.message)

/** The microphone side of a recording. `start` waits for the first live PCM and throws [VoiceRefused] or [VoiceNativeFailure]; a later end of the take goes to `onFailure`. `peak` is the linear peak 0 to 1 since the last call. */
internal interface VoiceCapture {
  suspend fun start(file: File, maxDurationMs: ULong, ticket: RecordingTicket, onLimit: () -> Unit, onFailure: (Exception) -> Unit)

  fun peak(): Float

  fun stop()

  /** Ends the take and gives the length in ms of the audio written to the file, or null when the capture does not measure it. */
  suspend fun finish(): ULong? { stop(); return null }
}

internal fun RecordingRefusal.toVoice(): VoiceRefusal = when (this) {
  RecordingRefusal.IN_CALL -> VoiceRefusal.IN_CALL
  RecordingRefusal.INTERRUPTED -> VoiceRefusal.INTERRUPTED
  RecordingRefusal.MICROPHONE_SILENT -> VoiceRefusal.MICROPHONE_SILENT
  RecordingRefusal.PERMISSION_DENIED -> VoiceRefusal.PERMISSION_DENIED
  RecordingRefusal.MICROPHONE_BUSY -> VoiceRefusal.MICROPHONE_BUSY
}

private fun voiceError(reason: VoiceRefusal) = MediaException.Voice(voiceRefusalText(reason), reason)

/**
 * Records a voice message into an m4a file (AAC, mono). [levels] gives the wave level 0 to 1 while a recording runs and nothing in rest.
 * The audio focus is taken for recording through the open layer and given back when the recording ends. A refused permission, a busy microphone
 * and an empty recording are a [MediaException.Voice] with the text of the core. The recording ends by itself after [maxDurationMs].
 */
class MediaVoiceRecorder internal constructor(
  private val capture: VoiceCapture,
  private val maxDurationMs: ULong,
  private val link: () -> AudioLink? = { Media.link },
  private val clock: () -> ULong = { SystemClock.elapsedRealtime().toULong() },
  private val levelIntervalMs: Long = LEVEL_INTERVAL_MS,
  private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Default),
  private val minTakeMs: ULong = 0uL,
  screen: ScreenAwake = ScreenAwake.shared
) {
  constructor(context: Context, maxDurationMs: Long? = null, minTakeMs: ULong = 0uL) : this(SystemVoiceCapture(knowing(context)), voiceLimitMs(maxDurationMs?.toULong()), minTakeMs = minTakeMs)

  private val wave = MutableSharedFlow<Float>(extraBufferCapacity = 1, onBufferOverflow = BufferOverflow.DROP_OLDEST)
  private val currentState = MutableStateFlow(VoiceState(0uL, VoicePhase.IDLE, null, 0uL))
  private val awake = ScreenAwakeHold(ScreenHoldKind.ALWAYS, screen)
  private fun publish(value: VoiceState) {
    currentState.value = value
    awake.set(value.phase != VoicePhase.IDLE)
  }
  val state: StateFlow<VoiceState> = currentState
  val states: StateFlow<VoiceState> = state
  private val lock = Any()
  private var running: Running? = null
  private var ended: Result<VoiceRecording>? = null
  private var heldAudio: RecordingTicket? = null
  private var revoked: java.io.Closeable? = null
  private var finishing: Deferred<Result<VoiceRecording>>? = null
  private var phase: StartPhase = StartPhase.Free
  private class Running(val file: File, val startedAt: ULong, var take: ULong = 0uL, var poll: Job? = null, var finish: Deferred<Result<VoiceRecording>>? = null)

  private sealed interface StartPhase {
    object Free : StartPhase
    object Starting : StartPhase
    object Limited : StartPhase
    class Failed(val error: Exception) : StartPhase
  }

  val levels: Flow<Float> = flow {
    val initial = currentState.value
    emitAll(channelFlow {
      val collector = launch(start = kotlinx.coroutines.CoroutineStart.UNDISPATCHED) { wave.collect { send(it) } }
      val take = if (initial.phase == VoicePhase.RECORDING) initial.take else currentState.first { it.phase == VoicePhase.RECORDING }.take
      currentState.first { it.take != take || it.outcome != null }
      collector.cancel()
    })
  }

  val isRecording: Boolean get() = synchronized(lock) { running != null }

  suspend fun start(file: File) {
    synchronized(lock) {
      if (running != null || finishing != null || phase !== StartPhase.Free) throw voiceError(VoiceRefusal.MICROPHONE_BUSY)
      phase = StartPhase.Starting
      ended = null
    }
    try {
      begin(file)
    } catch (refused: MediaException.Voice) {
      synchronized(lock) { publish(VoiceState(currentState.value.take, VoicePhase.IDLE, VoiceOutcome.Refused(refused.reason), 0uL)) }
      throw refused
    } finally {
      synchronized(lock) { phase = StartPhase.Free }
    }
  }

  private suspend fun begin(file: File) {
    val session = link() ?: throw MediaException.Closed()
    val hold = when (val admission = session.admit(RecordingKind.VOICE)) {
      is Admission.Admitted -> admission.ticket
      is Admission.Refused -> throw voiceError(admission.reason.toVoice())
    }
    lateinit var current: Running
    val limited = try {
      capture.start(file, maxDurationMs, hold, ::limitReached, ::captureFailed)
      // The clock of the take starts with its first PCM: `start` returns after it, not before the hardware opened.
      current = Running(file, clock())
      hold.start {}?.let { throw VoiceRefused(it.toVoice()) }
      commit(current, hold)
    } catch (error: Throwable) {
      try { capture.stop() } catch (stopping: Exception) { Media.recordNativeFailure(FailureSource.RECORDER, stopping) }
      hold.close()
      deleteMediaFile(file)
      throw when (error) {
        is VoiceRefused -> voiceError(error.reason)
        is VoiceNativeFailure -> failed(error.failure)
        else -> error
      }
    }
    if (limited) scope.launch(Dispatchers.IO) { finish(current) }
  }

  private fun failed(failure: NativeFailure): MediaException {
    Media.recordNativeFailure(failure)
    synchronized(lock) { publish(VoiceState(currentState.value.take, VoicePhase.IDLE, VoiceOutcome.Failed(failure), 0uL)) }
    return MediaException.Unexpected("${failure.domain}: ${failure.message}")
  }

  // One lock holds the end of the start phase, the running take and its RECORDING state: an early limit or loss is the result of start.
  private fun commit(current: Running, hold: RecordingTicket): Boolean = synchronized(lock) {
    (phase as? StartPhase.Failed)?.let { throw it.error }
    val limited = phase === StartPhase.Limited
    phase = StartPhase.Free
    heldAudio = hold
    running = current
    current.take = currentState.value.take + 1uL
    publish(VoiceState(current.take, VoicePhase.RECORDING, null, 0uL))
    current.poll = scope.launch { pollLevels() }
    revoked = hold.onRevoked { reason, done ->
      scope.launch(Dispatchers.IO) { try { finish(current, voiceError(reason.toVoice())) } finally { done() } }
    }
    limited
  }

  /** Ends the recording and gives the file. After the limit ended it, gives the outcome of that recording. */
  suspend fun stop(): VoiceRecording {
    val (current, task, done) = synchronized(lock) { Triple(running, finishing, ended) }
    if (current != null) return finish(current).getOrThrow()
    if (task != null) return task.await().getOrThrow()
    return (done ?: throw voiceError(VoiceRefusal.EMPTY)).getOrThrow()
  }

  private fun limitReached() {
    val current = synchronized(lock) { running.also { if (it == null && phase === StartPhase.Starting) phase = StartPhase.Limited } } ?: return
    scope.launch(Dispatchers.IO) { finish(current) }
  }

  private fun captureFailed(error: Exception) {
    val current = synchronized(lock) { running.also { if (it == null && phase === StartPhase.Starting) phase = StartPhase.Failed(error) } } ?: return
    scope.launch(Dispatchers.IO) {
      when (error) {
        is VoiceRefused -> finish(current, voiceError(error.reason))
        is VoiceNativeFailure -> finish(current, failure = error.failure)
        else -> finish(current, failure = voiceFailure(error))
      }
    }
  }

  private suspend fun finish(current: Running, interrupted: MediaException? = null, failure: NativeFailure? = null): Result<VoiceRecording> {
    val task = synchronized(lock) {
      current.finish ?: run {
        running = null
        try { revoked?.close() } catch (error: Exception) { Media.recordNativeFailure(FailureSource.RECORDER, error) }
        revoked = null
        current.poll?.cancel()
        val session = heldAudio
        heldAudio = null
        scope.async(Dispatchers.IO) { finishRecording(current, session, interrupted, failure) }.also { current.finish = it; finishing = it }
      }
    }
    onTestChange?.invoke()
    return task.await()
  }

  private suspend fun finishRecording(current: Running, session: RecordingTicket?, interrupted: MediaException?, failure: NativeFailure?): Result<VoiceRecording> {
    val elapsed = clock() - current.startedAt
    var length = elapsed
    var native = failure
    failure?.let { Media.recordNativeFailure(it) }
    val outcome = runCatching {
      try {
        if (interrupted != null || failure != null) capture.stop() else capture.finish()?.let { length = it }
      } catch (error: VoiceNativeFailure) {
        if (interrupted == null && failure == null) {
          native = error.failure
          Media.recordNativeFailure(error.failure)
          throw MediaException.Unexpected(error.failure.message)
        }
      } catch (error: VoiceRefused) {
        if (interrupted == null) throw voiceError(error.reason)
      } finally {
        session?.close()
      }
      if (interrupted != null) throw interrupted
      if (failure != null) throw MediaException.Unexpected(failure.message)
      if (voiceTooShort(length, minTakeMs)) throw voiceError(VoiceRefusal.TOO_SHORT)
      if (length == 0uL || !current.file.isFile || current.file.length() == 0L) throw voiceError(VoiceRefusal.EMPTY)
      VoiceRecording(current.file, length.toLong())
    }
    outcome.exceptionOrNull()?.let { error ->
      if (native == null && error !is MediaException.Voice && error !== interrupted) Media.recordNativeFailure(FailureSource.RECORDER, error)
    }
    if (outcome.isFailure) deleteMediaFile(current.file)
    val published = when {
      native != null -> VoiceOutcome.Failed(native!!)
      outcome.isSuccess -> outcome.getOrThrow().let { VoiceOutcome.Recorded(it.file.absolutePath, it.durationMs.toULong()) }
      outcome.exceptionOrNull() is MediaException.Voice -> VoiceOutcome.Refused((outcome.exceptionOrNull() as MediaException.Voice).reason)
      else -> VoiceOutcome.Failed(voiceFailure(outcome.exceptionOrNull()!!))
    }
    // The end of the take, its outcome and the room for the next start are one step: a start cannot commit between them.
    synchronized(lock) {
      ended = outcome
      finishing = null
      publish(VoiceState(current.take, VoicePhase.IDLE, published, 0uL))
    }
    onTestChange?.invoke()
    return outcome
  }

  private suspend fun pollLevels() {
    while (scope.isActive) {
      delay(levelIntervalMs)
      try {
        wave.tryEmit(voiceLevel(capture.peak()))
        synchronized(lock) {
          running?.let { publish(VoiceState(currentState.value.take, VoicePhase.RECORDING, null, clock() - it.startedAt)) }
        }
      } catch (error: Exception) {
        val failure = NativeFailure(FailureSource.RECORDER, error.javaClass.name, -1L, error.message ?: error.toString())
        synchronized(lock) { running }?.let { finish(it, failure = failure) }
        return
      }
    }
  }

  private companion object {
    const val LEVEL_INTERVAL_MS = 50L
  }
}

/** The budget of the core for one take, the way the camera asks it: one hold before the input opens. */
private suspend fun reserveCaptureBytes(bytes: Long): AutoCloseable {
  val hold = (ProcessMediaEnvironment.current ?: throw MediaException.Closed()).media.reserveCapture(bytes.toULong())
  return AutoCloseable { hold.release() }
}

internal class SystemVoiceCapture(
  private val context: Context,
  private val link: () -> AudioLink? = { Media.link },
  private val writer: () -> VoiceFileWriter = { MediaCodecVoiceFileWriter() },
  private val reserve: suspend (Long) -> AutoCloseable = ::reserveCaptureBytes,
  private val microphone: (RecordingAudioListener, (Long) -> AutoCloseable) -> CameraMicrophone = { listener, lease -> RecordingAudio(listener, lease, context.mainExecutor).asMicrophone() }
) : VoiceCapture {
  // A lost take has closed its input at once; the marker stays so that the end of that take is still told as the loss.
  private class Active(val input: VoiceInput, val done: CompletableDeferred<Long?>, val release: () -> Unit) { @Volatile var lost = false }

  private val lock = Any()
  private var active: Active? = null

  override suspend fun start(file: File, maxDurationMs: ULong, ticket: RecordingTicket, onLimit: () -> Unit, onFailure: (Exception) -> Unit) {
    val audio = link() ?: throw MediaException.Closed()
    val settings = voiceEncoderSettings()
    val spec = PcmSpec(MediaRecorder.AudioSource.MIC, settings.sampleRateHz.toInt(), if (settings.channels.toInt() == 1) AudioFormat.CHANNEL_IN_MONO else AudioFormat.CHANNEL_IN_STEREO, AudioFormat.ENCODING_PCM_16BIT)
    val cost = try { pcmSizes(spec).lease } catch (error: PcmSizeError) { throw VoiceNativeFailure(NativeFailure(FailureSource.RECORDER, "android.media.AudioRecord", error.code, error.message ?: "bad PCM spec")) }
    val hold = try { reserve(cost) } catch (error: Exception) {
      if (error is kotlinx.coroutines.CancellationException) throw error
      throw VoiceNativeFailure(NativeFailure(FailureSource.RECORDER, error.javaClass.name, 0L, error.message ?: error.toString()))
    }
    val memory = CaptureMemory(cost)
    val release = { memory.close { hold.close() } }
    val first = CompletableDeferred<Unit>()
    val done = CompletableDeferred<Long?>()
    fun ended(error: Exception) {
      if (!first.completeExceptionally(error)) onFailure(error)
      done.completeExceptionally(error)
    }
    lateinit var mine: Active
    val listener = object : VoiceTakeListener {
      override fun onStarted() { first.complete(Unit) }
      override fun onLimit() = onLimit()
      override fun onFinished(file: File, durationUs: Long) { done.complete(durationUs / 1_000L) }
      override fun onFailed(failure: NativeFailure) = ended(VoiceNativeFailure(failure))
      override fun onCancelled() { first.completeExceptionally(kotlinx.coroutines.CancellationException("the take was cancelled")); done.complete(null) }
      override fun onLost() { mine.lost = true; ended(VoiceRefused(VoiceRefusal.MICROPHONE_SILENT)) }
    }
    val input = VoiceInput(audio, spec, settings, maxDurationMs.toLong() * 1_000L, file, writer(), listener) { heard ->
      val made = microphone(heard, memory::lease)
      // The hold goes back after the last lease of the microphone: the owner closes it at the end of the take or of its restore.
      object : CameraMicrophone by made { override fun close() { try { made.close() } finally { release() } } }
    }
    mine = Active(input, done, release)
    // Registered before the owner can act: its first word may come at once.
    val previous = synchronized(lock) { active.also { active = mine } }
    previous?.input?.cancel()
    val revoked = ticket.onRevoked { reason, ack -> first.completeExceptionally(VoiceRefused(reason.toVoice())); ack() }
    try {
      input.start()
      first.await()
    } catch (error: Throwable) {
      if (!mine.lost) { leave(); input.cancel(); release() }
      throw error
    } finally {
      revoked.close()
    }
  }

  override fun peak(): Float = synchronized(lock) { active }?.input?.peak() ?: 0f

  override fun stop() { leave()?.takeUnless { it.lost }?.let { it.input.cancel(); it.release() } }

  override suspend fun finish(): ULong? {
    val taking = leave() ?: return null
    if (taking.lost) throw VoiceRefused(VoiceRefusal.MICROPHONE_SILENT)
    try {
      taking.input.finish()
      return taking.done.await()?.toULong()
    } finally {
      taking.release()
    }
  }

  private fun leave(): Active? = synchronized(lock) { active?.also { if (!it.lost) active = null } }
}
