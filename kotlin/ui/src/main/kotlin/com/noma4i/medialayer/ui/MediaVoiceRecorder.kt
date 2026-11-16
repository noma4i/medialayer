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
import com.noma4i.medialayer.RecordingKind
import com.noma4i.medialayer.RecordingRefusal
import com.noma4i.medialayer.VoiceEncoderSettings
import com.noma4i.medialayer.voiceEncoderSettings
import com.noma4i.medialayer.voiceLevel
import com.noma4i.medialayer.voiceLimitMs
import com.noma4i.medialayer.voiceRefusalText
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
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** A finished voice recording: an m4a file (AAC, mono) and its length. */
data class VoiceRecording(val file: File, val durationMs: Long)

internal class VoiceRefused(val reason: VoiceRefusal) : Exception()

/** The microphone side of a recording. `start` throws [VoiceRefused], `peak` is the linear peak 0 to 1 since the last call. */
internal interface VoiceCapture {
  fun start(file: File, maxDurationMs: ULong, onLimit: () -> Unit)

  fun peak(): Float

  fun stop()

  fun isSilenced(): Boolean = false

  fun onSilenced(listener: () -> Unit) = Unit
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
  private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
) {
  constructor(context: Context, maxDurationMs: Long? = null) : this(SystemVoiceCapture(context.applicationContext), voiceLimitMs(maxDurationMs?.toULong()))

  private val wave = MutableSharedFlow<Float>(extraBufferCapacity = 1, onBufferOverflow = BufferOverflow.DROP_OLDEST)
  private val lock = Any()
  private var running: Running? = null
  private var ended: Result<VoiceRecording>? = null
  private var heldAudio: RecordingTicket? = null
  private var revoked: java.io.Closeable? = null
  private var starting = false
  private class Running(val file: File, val startedAt: ULong, var poll: Job? = null)

  val levels: Flow<Float> = wave.asSharedFlow()

  val isRecording: Boolean get() = synchronized(lock) { running != null }

  suspend fun start(file: File) {
    synchronized(lock) {
      check(running == null && !starting) { "a recording is already running" }
      starting = true
      ended = null
    }
    try {
      withContext(Dispatchers.IO) { begin(file) }
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
    try {
      val refusal = hold.start {
        capture.start(file, maxDurationMs, ::limitReached)
        synchronized(lock) {
          heldAudio = hold
          val current = Running(file, clock()).also {
            running = it
            it.poll = scope.launch { pollLevels() }
          }
          revoked = hold.onRevoked { reason -> scope.launch(Dispatchers.IO) { finish(current, voiceError(reason)) } }
        }
      }
      if (refusal != null) throw voiceError(refusal)
    } catch (refused: VoiceRefused) {
      hold.close()
      file.delete()
      throw MediaException.Voice(voiceRefusalText(refused.reason))
    } catch (failure: Throwable) {
      hold.close()
      file.delete()
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

  private fun voiceError(reason: RecordingRefusal) = MediaException.Voice(voiceRefusalText(when (reason) {
    RecordingRefusal.IN_CALL -> VoiceRefusal.IN_CALL
    RecordingRefusal.INTERRUPTED -> VoiceRefusal.INTERRUPTED
    RecordingRefusal.MICROPHONE_SILENT -> VoiceRefusal.MICROPHONE_SILENT
    RecordingRefusal.PERMISSION_DENIED -> VoiceRefusal.PERMISSION_DENIED
    RecordingRefusal.MICROPHONE_BUSY -> VoiceRefusal.MICROPHONE_BUSY
  }))

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

  private fun finish(current: Running, interrupted: MediaException? = null): Result<VoiceRecording> = synchronized(lock) {
    if (running !== current) return@synchronized checkNotNull(ended)
    running = null
    revoked?.close()
    revoked = null
    current.poll?.cancel()
    val session = heldAudio
    heldAudio = null
    val length = clock() - current.startedAt
    val outcome = runCatching {
      try {
        capture.stop()
      } catch (refused: VoiceRefused) {
        if (interrupted == null) throw MediaException.Voice(voiceRefusalText(refused.reason))
      } finally {
        session?.close()
      }
      if (interrupted != null) throw interrupted
      if (length == 0uL || !current.file.isFile || current.file.length() == 0L) throw MediaException.Voice(voiceRefusalText(VoiceRefusal.EMPTY))
      VoiceRecording(current.file, length.toLong())
    }
    if (outcome.isFailure) current.file.delete()
    ended = outcome
    onTestChange?.invoke()
    outcome
  }

  private suspend fun pollLevels() {
    while (scope.isActive) {
      delay(levelIntervalMs)
      wave.tryEmit(voiceLevel(capture.peak()))
    }
  }

  private companion object {
    const val LEVEL_INTERVAL_MS = 50L
  }
}

internal class SystemVoiceCapture(private val context: Context) : VoiceCapture {
  private var recorder: MediaRecorder? = null
  private var silence: () -> Unit = {}
  private val recordingCallback = object : AudioManager.AudioRecordingCallback() {
    override fun onRecordingConfigChanged(configs: MutableList<AudioRecordingConfiguration>) {
      val session = recorder?.activeRecordingConfiguration?.clientAudioSessionId
      if (session != null && configs.any { it.clientAudioSessionId == session && it.isClientSilenced }) silence()
    }
  }

  override fun onSilenced(listener: () -> Unit) { silence = listener }

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
      made.prepare()
      made.start()
    } catch (failure: IOException) {
      made.release()
      throw VoiceRefused(VoiceRefusal.MICROPHONE_BUSY)
    } catch (failure: RuntimeException) {
      made.release()
      throw VoiceRefused(VoiceRefusal.MICROPHONE_BUSY)
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
      made.unregisterAudioRecordingCallback(recordingCallback)
      made.release()
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
