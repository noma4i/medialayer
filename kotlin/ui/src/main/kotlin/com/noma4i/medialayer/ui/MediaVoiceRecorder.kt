package com.noma4i.medialayer.ui

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.media.MediaRecorder
import android.media.AudioManager
import android.media.AudioRecordingConfiguration
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
import com.noma4i.medialayer.VoiceEncoderSettings
import com.noma4i.medialayer.voiceEncoderSettings
import com.noma4i.medialayer.voiceLevel
import com.noma4i.medialayer.voiceLimitMs
import com.noma4i.medialayer.voiceRefusalText
import com.noma4i.medialayer.voiceTooShort
import java.io.File
import java.io.IOException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
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
import kotlinx.coroutines.withContext

/** A finished voice recording: an m4a file (AAC, mono) and its length. */
data class VoiceRecording(val file: File, val durationMs: Long)

internal class VoiceRefused(val reason: VoiceRefusal, val native: Throwable? = null) : Exception()

/** The microphone side of a recording. `start` throws [VoiceRefused], `peak` is the linear peak 0 to 1 since the last call. */
internal interface VoiceCapture {
  fun start(file: File, maxDurationMs: ULong, onLimit: () -> Unit)

  fun peak(): Float

  fun stop()

  fun isSilenced(): Boolean = false

  fun onSilenced(listener: () -> Unit) = Unit

  fun onFailure(listener: (NativeFailure) -> Unit) = Unit
}

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
  private val minTakeMs: ULong = 0uL
) {
  constructor(context: Context, maxDurationMs: Long? = null, minTakeMs: ULong = 0uL) : this(SystemVoiceCapture(context.applicationContext), voiceLimitMs(maxDurationMs?.toULong()), minTakeMs = minTakeMs)

  private val wave = MutableSharedFlow<Float>(extraBufferCapacity = 1, onBufferOverflow = BufferOverflow.DROP_OLDEST)
  private val currentState = MutableStateFlow(VoiceState(0uL, VoicePhase.IDLE, null, 0uL))
  val state: StateFlow<VoiceState> = currentState
  val states: StateFlow<VoiceState> = state
  private val lock = Any()
  private var running: Running? = null
  private var ended: Result<VoiceRecording>? = null
  private var heldAudio: RecordingTicket? = null
  private var revoked: java.io.Closeable? = null
  private var starting = false
  private class Running(val file: File, val startedAt: ULong, var poll: Job? = null)

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
      check(running == null && !starting) { "a recording is already running" }
      starting = true
      ended = null
    }
    try {
      withContext(Dispatchers.IO) { begin(file) }
    } catch (cancelled: kotlinx.coroutines.CancellationException) {
      synchronized(lock) { running }?.let { finish(it, voiceError(RecordingRefusal.INTERRUPTED)) }
      throw cancelled
    } catch (refused: MediaException.Voice) {
      synchronized(lock) { currentState.value = VoiceState(currentState.value.take, VoicePhase.IDLE, VoiceOutcome.Refused(refused.reason), 0uL) }
      throw refused
    } finally {
      synchronized(lock) { starting = false }
    }
  }

  private suspend fun begin(file: File) {
    val session = link() ?: throw MediaException.Closed()
    val hold = when (val admission = session.admit(RecordingKind.VOICE)) {
      is Admission.Admitted -> admission.ticket
      is Admission.Refused -> throw voiceError(admission.reason)
    }
    capture.onSilenced { session.send(com.noma4i.medialayer.AudioInput.Silenced(hold.id)) }
    capture.onFailure { failure ->
      Media.recordNativeFailure(failure)
      synchronized(lock) { running }?.let { finish(it, nativeFailure = failure) }
    }
    try {
      val refusal = hold.start {
        capture.start(file, maxDurationMs, ::limitReached)
        synchronized(lock) {
          heldAudio = hold
          val current = Running(file, clock()).also {
            running = it
            currentState.value = VoiceState(currentState.value.take + 1uL, VoicePhase.RECORDING, null, 0uL)
            it.poll = scope.launch { pollLevels() }
          }
          revoked = hold.onRevoked { reason, done ->
            scope.launch(Dispatchers.IO) { try { finish(current, voiceError(reason)) } finally { done() } }
          }
        }
      }
      if (refusal != null) throw voiceError(refusal)
    } catch (refused: VoiceRefused) {
      refused.native?.let { Media.recordNativeFailure(FailureSource.RECORDER, it) }
      hold.close()
      deleteMediaFile(file)
      throw MediaException.Voice(voiceRefusalText(refused.reason), refused.reason)
    } catch (failure: Throwable) {
      hold.close()
      deleteMediaFile(file)
      throw failure
    }
    synchronized(lock) {
      if (capture.isSilenced()) {
        val failure = voiceError(RecordingRefusal.MICROPHONE_SILENT)
        running?.let { finish(it, failure) }
        throw failure
      }
    }
  }

  private fun voiceError(reason: RecordingRefusal): MediaException.Voice {
    val voiceReason = when (reason) {
    RecordingRefusal.IN_CALL -> VoiceRefusal.IN_CALL
    RecordingRefusal.INTERRUPTED -> VoiceRefusal.INTERRUPTED
    RecordingRefusal.MICROPHONE_SILENT -> VoiceRefusal.MICROPHONE_SILENT
    RecordingRefusal.PERMISSION_DENIED -> VoiceRefusal.PERMISSION_DENIED
    RecordingRefusal.MICROPHONE_BUSY -> VoiceRefusal.MICROPHONE_BUSY
    }
    return MediaException.Voice(voiceRefusalText(voiceReason), voiceReason)
  }

  /** Ends the recording and gives the file. After the limit ended it, gives the outcome of that recording. */
  suspend fun stop(): VoiceRecording {
    val current = synchronized(lock) { running }
    if (current == null) return (synchronized(lock) { ended } ?: throw IllegalStateException("no recording was started")).getOrThrow()
    return withContext(Dispatchers.IO) { finish(current).getOrThrow() }
  }

  private fun limitReached() {
    val current = synchronized(lock) { running } ?: return
    scope.launch(Dispatchers.IO) { finish(current) }
  }

  private fun finish(current: Running, interrupted: MediaException? = null, nativeFailure: NativeFailure? = null): Result<VoiceRecording> = synchronized(lock) {
    if (running !== current) return@synchronized checkNotNull(ended)
    running = null
    try { revoked?.close() } catch (error: Exception) { Media.recordNativeFailure(FailureSource.RECORDER, error) }
    revoked = null
    current.poll?.cancel()
    val session = heldAudio
    heldAudio = null
    val length = clock() - current.startedAt
    val outcome = runCatching {
      try {
        capture.stop()
      } catch (refused: VoiceRefused) {
        refused.native?.let { Media.recordNativeFailure(FailureSource.RECORDER, it) }
        if (refused.reason == VoiceRefusal.EMPTY && interrupted == null && nativeFailure == null && voiceTooShort(length, minTakeMs)) {
          throw MediaException.Voice(voiceRefusalText(VoiceRefusal.TOO_SHORT), VoiceRefusal.TOO_SHORT)
        }
        if (interrupted == null) throw MediaException.Voice(voiceRefusalText(refused.reason), refused.reason)
      } finally {
        session?.close()
      }
      if (interrupted != null) throw interrupted
      if (nativeFailure != null) throw MediaException.Unexpected(nativeFailure.message)
      if (voiceTooShort(length, minTakeMs)) throw MediaException.Voice(voiceRefusalText(VoiceRefusal.TOO_SHORT), VoiceRefusal.TOO_SHORT)
      if (length == 0uL || !current.file.isFile || current.file.length() == 0L) throw MediaException.Voice(voiceRefusalText(VoiceRefusal.EMPTY), VoiceRefusal.EMPTY)
      VoiceRecording(current.file, length.toLong())
    }
    outcome.exceptionOrNull()?.let { error ->
      if (nativeFailure == null && error !is MediaException.Voice && error !== interrupted) Media.recordNativeFailure(FailureSource.RECORDER, error)
    }
    if (outcome.isFailure) deleteMediaFile(current.file)
    ended = outcome
    val published = when {
      nativeFailure != null -> VoiceOutcome.Failed(nativeFailure)
      outcome.isSuccess -> outcome.getOrThrow().let { VoiceOutcome.Recorded(it.file.absolutePath, it.durationMs.toULong()) }
      outcome.exceptionOrNull() is MediaException.Voice -> VoiceOutcome.Refused((outcome.exceptionOrNull() as MediaException.Voice).reason)
      else -> VoiceOutcome.Failed(NativeFailure(FailureSource.RECORDER, "android.media.MediaRecorder", -1L, outcome.exceptionOrNull().toString()))
    }
    currentState.value = VoiceState(currentState.value.take, VoicePhase.IDLE, published, 0uL)
    onTestChange?.invoke()
    outcome
  }

  private suspend fun pollLevels() {
    while (scope.isActive) {
      delay(levelIntervalMs)
      try {
        wave.tryEmit(voiceLevel(capture.peak()))
        synchronized(lock) {
          running?.let { currentState.value = VoiceState(currentState.value.take, VoicePhase.RECORDING, null, clock() - it.startedAt) }
        }
      } catch (error: Exception) {
        val failure = NativeFailure(FailureSource.RECORDER, error.javaClass.name, -1L, error.message ?: error.toString())
        Media.recordNativeFailure(failure)
        synchronized(lock) { running }?.let { finish(it, nativeFailure = failure) }
        return
      }
    }
  }

  private companion object {
    const val LEVEL_INTERVAL_MS = 50L
  }
}

internal class SystemVoiceCapture(private val context: Context) : VoiceCapture {
  private var recorder: MediaRecorder? = null
  private var silence: () -> Unit = {}
  private var failure: (NativeFailure) -> Unit = {}
  private val recordingCallback = object : AudioManager.AudioRecordingCallback() {
    override fun onRecordingConfigChanged(configs: MutableList<AudioRecordingConfiguration>) {
      val session = recorder?.activeRecordingConfiguration?.clientAudioSessionId
      if (session != null && configs.any { it.clientAudioSessionId == session && it.isClientSilenced }) silence()
    }
  }

  override fun onSilenced(listener: () -> Unit) { silence = listener }
  override fun onFailure(listener: (NativeFailure) -> Unit) { failure = listener }

  override fun start(file: File, maxDurationMs: ULong, onLimit: () -> Unit) {
    if (context.checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) throw VoiceRefused(VoiceRefusal.PERMISSION_DENIED)
    val made = MediaRecorder(context)
    try {
      made.setAudioSource(MediaRecorder.AudioSource.MIC)
      made.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4)
      made.setAudioEncoder(MediaRecorder.AudioEncoder.AAC)
      applyVoiceEncoder(object : EncoderTarget {
        override fun setChannels(count: Int) = made.setAudioChannels(count)
        override fun setSampleRate(hertz: Int) = made.setAudioSamplingRate(hertz)
        override fun setBitRate(bitsPerSecond: Int) = made.setAudioEncodingBitRate(bitsPerSecond)
      })
      made.setMaxDuration(maxDurationMs.toInt())
      made.setOutputFile(file.path)
      made.registerAudioRecordingCallback(context.mainExecutor, recordingCallback)
      made.setOnInfoListener { _, what, _ -> if (what == MediaRecorder.MEDIA_RECORDER_INFO_MAX_DURATION_REACHED) onLimit() }
      made.setOnErrorListener { _, what, extra -> failure(NativeFailure(FailureSource.RECORDER, "android.media.MediaRecorder", what.toLong(), "MediaRecorder error $what ($extra)")) }
      made.prepare()
      made.start()
    } catch (failure: IOException) {
      made.release()
      throw VoiceRefused(VoiceRefusal.MICROPHONE_BUSY, failure)
    } catch (failure: RuntimeException) {
      made.release()
      throw VoiceRefused(VoiceRefusal.MICROPHONE_BUSY, failure)
    }
    recorder = made
  }

  override fun peak(): Float = (recorder?.maxAmplitude ?: 0) / MAX_AMPLITUDE

  override fun isSilenced(): Boolean = recorder?.activeRecordingConfiguration?.isClientSilenced == true

  override fun stop() {
    val made = recorder ?: return
    recorder = null
    try {
      made.stop()
    } catch (failure: RuntimeException) {
      throw VoiceRefused(VoiceRefusal.EMPTY)
    } finally {
      try { made.unregisterAudioRecordingCallback(recordingCallback) } finally { made.release() }
    }
  }

  private companion object {
    const val MAX_AMPLITUDE = 32_767f
  }
}

/** The encoder calls of a recorder, so the voice setup of the core can be read without a microphone. */
internal interface EncoderTarget {
  fun setChannels(count: Int)

  fun setSampleRate(hertz: Int)

  fun setBitRate(bitsPerSecond: Int)
}

internal fun applyVoiceEncoder(target: EncoderTarget, settings: VoiceEncoderSettings = voiceEncoderSettings()) {
  target.setChannels(settings.channels.toInt())
  target.setSampleRate(settings.sampleRateHz.toInt())
  target.setBitRate(settings.bitRate.toInt())
}
