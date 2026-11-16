package com.noma4i.medialayer.ui

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.media.MediaRecorder
import android.os.SystemClock
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.VoiceRefusal
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
  fun start(file: File, maxDurationMs: Long, onLimit: () -> Unit)

  fun peak(): Float

  fun stop()

  fun isSilenced(): Boolean = false
}

/**
 * Records a voice message into an m4a file (AAC, mono). [levels] gives the wave level 0 to 1 while a recording runs and nothing in rest.
 * The audio focus is taken for recording through [Media.audio] and given back when the recording ends. A refused permission, a busy microphone
 * and an empty recording are a [MediaException.Voice] with the text of the core. The recording ends by itself after [maxDurationMs].
 */
class MediaVoiceRecorder internal constructor(
  private val capture: VoiceCapture,
  private val maxDurationMs: Long,
  private val audio: () -> MediaAudioSession? = { Media.audio },
  private val clock: () -> Long = SystemClock::elapsedRealtime,
  private val levelIntervalMs: Long = LEVEL_INTERVAL_MS,
  private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Default),
  private val calls: CallGate = Media.calls
) {
  constructor(context: Context, maxDurationMs: Long? = null) : this(SystemVoiceCapture(context.applicationContext), voiceLimitMs(maxDurationMs?.toULong()).toLong())

  private val wave = MutableSharedFlow<Float>(extraBufferCapacity = 1, onBufferOverflow = BufferOverflow.DROP_OLDEST)
  private val lock = Any()
  private var running: Running? = null
  private var ended: Result<VoiceRecording>? = null
  private var heldAudio: MediaRecordingHold? = null
  private var starting = false
  private val callListener = CallListener(::callBegan)
  private val microphoneListener: () -> Unit = { checkSilenced() }

  private class Running(val file: File, val startedAt: Long, var poll: Job? = null)

  val levels: Flow<Float> = wave.asSharedFlow()

  val isRecording: Boolean get() = synchronized(lock) { running != null }

  fun start(file: File) {
    synchronized(lock) {
      check(running == null && !starting) { "a recording is already running" }
      starting = true
      ended = null
    }
    try {
      begin(file)
    } finally {
      synchronized(lock) { starting = false }
    }
  }

  private fun begin(file: File) {
    val session = audio()
    val availability = session?.microphoneAvailability?.value ?: MicrophoneAvailability.AVAILABLE
    if (availability != MicrophoneAvailability.AVAILABLE) throw unavailable(availability)
    if (calls.inCall) throw inCall()
    val hold = session?.holdRecording()
    if (session != null && hold == null) throw unavailable(session.microphoneAvailability.value)
    try {
      capture.start(file, maxDurationMs, ::limitReached)
    } catch (refused: VoiceRefused) {
      hold?.close()
      file.delete()
      val reason = if (refused.reason == VoiceRefusal.PERMISSION_DENIED) MicrophoneAvailability.PERMISSION_DENIED else MicrophoneAvailability.OTHER_APP
      session?.recordingFailed(reason)
      throw unavailable(session?.microphoneAvailability?.value ?: reason)
    } catch (failure: Throwable) {
      hold?.close()
      file.delete()
      throw failure
    }
    synchronized(lock) {
      heldAudio = hold
      val current = Running(file, clock()).also {
        running = it
        it.poll = scope.launch { pollLevels() }
      }
      if (!calls.register(callListener)) {
        finish(current, inCall())
        throw inCall()
      }
      session?.observeMicrophone(microphoneListener)
      if (capture.isSilenced()) {
        session?.recordingFailed()
        val failure = unavailable(MicrophoneAvailability.OTHER_APP)
        finish(current, failure)
        throw failure
      }
    }
  }

  private fun inCall() = MediaException.Voice(voiceRefusalText(VoiceRefusal.IN_CALL))

  private fun unavailable(value: MicrophoneAvailability) = MediaException.Voice(voiceRefusalText(value.voiceRefusal()))

  private fun callBegan() {
    val current = synchronized(lock) { running } ?: return
    scope.launch(Dispatchers.IO) { finish(current, inCall()) }
  }

  private fun checkSilenced() {
    val current = synchronized(lock) { running } ?: return
    if (!capture.isSilenced()) return
    val session = audio()
    session?.recordingFailed()
    scope.launch(Dispatchers.IO) { finish(current, unavailable(MicrophoneAvailability.OTHER_APP)) }
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

  private fun finish(current: Running, interrupted: MediaException? = null): Result<VoiceRecording> = synchronized(lock) {
    if (running !== current) return@synchronized checkNotNull(ended)
    running = null
    calls.unregister(callListener)
    audio()?.forgetMicrophone(microphoneListener)
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
      if (length <= 0 || !current.file.isFile || current.file.length() == 0L) throw MediaException.Voice(voiceRefusalText(VoiceRefusal.EMPTY))
      VoiceRecording(current.file, length)
    }
    if (outcome.isFailure) current.file.delete()
    ended = outcome
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

  override fun start(file: File, maxDurationMs: Long, onLimit: () -> Unit) {
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
