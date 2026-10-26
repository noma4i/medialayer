package com.noma4i.medialayer.ui

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.media.MediaRecorder
import android.os.SystemClock
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.VoiceRefusal
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
  private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
) {
  constructor(context: Context, maxDurationMs: Long? = null) : this(SystemVoiceCapture(context.applicationContext), voiceLimitMs(maxDurationMs?.toULong()).toLong())

  private val wave = MutableSharedFlow<Float>(extraBufferCapacity = 1, onBufferOverflow = BufferOverflow.DROP_OLDEST)
  private val lock = Any()
  private var running: Running? = null
  private var ended: Result<VoiceRecording>? = null
  private var heldAudio: MediaAudioSession? = null

  private class Running(val file: File, val startedAt: Long, var poll: Job? = null)

  val levels: Flow<Float> = wave.asSharedFlow()

  val isRecording: Boolean get() = synchronized(lock) { running != null }

  fun start(file: File) {
    synchronized(lock) {
      check(running == null) { "a recording is already running" }
      ended = null
    }
    val session = audio()
    if (session != null && !session.activateRecording()) throw MediaException.Voice(voiceRefusalText(VoiceRefusal.MICROPHONE_BUSY))
    try {
      capture.start(file, maxDurationMs, ::limitReached)
    } catch (refused: VoiceRefused) {
      session?.deactivateRecording()
      file.delete()
      throw MediaException.Voice(voiceRefusalText(refused.reason))
    }
    synchronized(lock) {
      heldAudio = session
      running = Running(file, clock()).also { it.poll = scope.launch { pollLevels() } }
    }
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

  private fun finish(current: Running): Result<VoiceRecording> = synchronized(lock) {
    if (running !== current) return@synchronized checkNotNull(ended)
    running = null
    current.poll?.cancel()
    val session = heldAudio
    heldAudio = null
    val length = clock() - current.startedAt
    val outcome = runCatching {
      try {
        capture.stop()
      } catch (refused: VoiceRefused) {
        throw MediaException.Voice(voiceRefusalText(refused.reason))
      } finally {
        session?.deactivateRecording()
      }
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
      made.setAudioChannels(1)
      made.setAudioSamplingRate(SAMPLE_RATE)
      made.setAudioEncodingBitRate(BIT_RATE)
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
    const val SAMPLE_RATE = 44_100
    const val BIT_RATE = 64_000
    const val MAX_AMPLITUDE = 32_767f
  }
}
