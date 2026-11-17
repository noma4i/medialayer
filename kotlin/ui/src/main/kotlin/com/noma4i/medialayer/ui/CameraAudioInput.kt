package com.noma4i.medialayer.ui

import com.noma4i.medialayer.FailureSource
import com.noma4i.medialayer.NativeFailure
import com.noma4i.medialayer.RecordingKind

internal interface CameraMicrophone : AutoCloseable {
  fun start(spec: PcmSpec): Long

  fun stop(generation: Long)
}

internal interface CameraAudioTap {
  fun onTimebase(generation: Long, originNanos: Long)

  fun onPcm(chunk: PcmChunk)
}

internal fun RecordingAudio.asMicrophone(): CameraMicrophone = object : CameraMicrophone {
  override fun start(spec: PcmSpec) = this@asMicrophone.start(spec)

  override fun stop(generation: Long) = this@asMicrophone.stop(generation)

  override fun close() = this@asMicrophone.close()
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
      live?.let { microphone.stop(it.audio) }
      live = null
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
      val current = live?.takeIf { it.core == generation } ?: return
      live = null
      microphone.stop(current.audio)
    }
  }

  override fun onLive(generation: Long) = whileLive(generation) { handle, core -> issue(CaptureAudioIssue.ACTIVE); handle.sound(core, true) }

  override fun onSilenced(generation: Long, silenced: Boolean) = whileLive(generation) { handle, core ->
    issue(if (silenced) CaptureAudioIssue.SILENCED else CaptureAudioIssue.ACTIVE)
    handle.sound(core, !silenced)
  }

  override fun onFailed(generation: Long, failure: NativeFailure) = whileLive(generation) { handle, core ->
    live = null
    failed(failure)
    issue(CaptureAudioIssue.SOURCE_ERROR)
    handle.restoreFailed(core)
  }

  override fun onTimebase(generation: Long, originNanos: Long) = whileLive(generation) { _, _ -> tap.onTimebase(generation, originNanos) }

  override fun onPcm(chunk: PcmChunk) = whileLive(chunk.generation) { _, _ -> tap.onPcm(chunk) }

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
      live?.let { microphone.stop(it.audio) }
      live = null
    }
    opened.close()
    microphone.close()
  }
}
