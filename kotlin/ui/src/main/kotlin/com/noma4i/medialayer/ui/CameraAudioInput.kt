package com.noma4i.medialayer.ui

import com.noma4i.medialayer.FailureSource
import com.noma4i.medialayer.NativeFailure
import com.noma4i.medialayer.RecordingKind

internal interface CameraMicrophone : AutoCloseable {
  fun start(spec: PcmSpec): Long

  fun stop(generation: Long)

  /** Returns once no AudioRecord of this microphone is live: the system has it back. */
  suspend fun idle() = Unit
}

internal interface CameraAudioTap {
  fun onTimebase(generation: Long, originNanos: Long)

  fun onPcm(chunk: PcmChunk)

  /** The source of this generation ended without a SILENCED word (stop, close, read failure, a restore): no PCM of it comes any more. */
  fun onSourceEnd(generation: Long) = Unit
}

internal fun RecordingAudio.asMicrophone(): CameraMicrophone = object : CameraMicrophone {
  override fun start(spec: PcmSpec) = this@asMicrophone.start(spec)

  override fun stop(generation: Long) = this@asMicrophone.stop(generation)

  override fun close() = this@asMicrophone.close()

  override suspend fun idle() = this@asMicrophone.idle()
}

internal class CameraAudioInput(
  link: AudioLink,
  private val spec: PcmSpec,
  private val tap: CameraAudioTap,
  private val issue: (CaptureAudioIssue) -> Unit,
  kind: RecordingKind = RecordingKind.VIDEO,
  private val failed: (NativeFailure) -> Unit = { Media.recordNativeFailure(it) },
  microphone: (RecordingAudioListener) -> CameraMicrophone
) : AudioInputOwner, RecordingAudioListener, AutoCloseable {
  private class Live(val core: ULong, val audio: Long)

  private val lock = Any()
  private val microphone = microphone(this)
  private var live: Live? = null
  private var closed = false
  private var input: AudioInputHandle? = null
  private val opened = link.openInput(kind, this)

  val audioGeneration: Long? get() = synchronized(lock) { live?.audio }

  override fun restore(input: AudioInputHandle, generation: ULong) {
    synchronized(lock) {
      if (closed) return
      this.input = input
      release()
      live = try { Live(generation, microphone.start(spec)) } catch (error: Exception) {
        failed(NativeFailure(FailureSource.RECORDER, error.javaClass.name, 0L, error.message ?: error.toString()))
        issue(CaptureAudioIssue.SOURCE_ERROR)
        input.restoreFailed(generation)
        null
      }
    }
  }

  override fun stop(generation: ULong) {
    synchronized(lock) {
      if (live?.core != generation) return
      release()
    }
  }

  override fun onLive(generation: Long) = whileLive(generation) { handle, core -> issue(CaptureAudioIssue.ACTIVE); handle.sound(core, true) }

  override fun onSilenced(generation: Long, silenced: Boolean) = whileLive(generation) { handle, core ->
    issue(if (silenced) CaptureAudioIssue.SILENCED else CaptureAudioIssue.ACTIVE)
    handle.sound(core, !silenced)
  }

  override fun onFailed(generation: Long, failure: NativeFailure) = whileLive(generation) { handle, core ->
    release(stopMicrophone = false)
    failed(failure)
    issue(CaptureAudioIssue.SOURCE_ERROR)
    handle.restoreFailed(core)
  }

  override fun onEnded(generation: Long) = whileLive(generation) { _, _ -> release(stopMicrophone = false) }

  override fun onTimebase(generation: Long, originNanos: Long) = whileLive(generation) { _, _ -> tap.onTimebase(generation, originNanos) }

  override fun onPcm(chunk: PcmChunk) = whileLive(chunk.generation) { _, _ -> tap.onPcm(chunk) }

  /** The one place a generation of the source ends: the take hears it once, whoever ended it. Runs under the lock. */
  private fun release(stopMicrophone: Boolean = true) {
    val current = live ?: return
    live = null
    if (stopMicrophone) microphone.stop(current.audio)
    tap.onSourceEnd(current.audio)
  }

  // Every word of the microphone runs under the lock that stop and close take: nothing reaches the core or the take after its generation ended.
  private fun whileLive(generation: Long, deliver: (AudioInputHandle, ULong) -> Unit) {
    synchronized(lock) {
      val current = live?.takeIf { it.audio == generation } ?: return
      deliver(input ?: return, current.core)
    }
  }

  override fun close() {
    synchronized(lock) {
      if (closed) return
      closed = true
      release()
    }
    opened.close()
    microphone.close()
  }
}
