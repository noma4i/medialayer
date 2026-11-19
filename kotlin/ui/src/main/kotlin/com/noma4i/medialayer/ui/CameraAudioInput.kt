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
  private val timer: RetryTimer = RetryTimer.system,
  /** The microphone of a run connected (its source started), or gave up: whoever waited for it goes on. */
  private val connected: () -> Unit = {},
  microphone: (RecordingAudioListener) -> CameraMicrophone
) : AudioInputOwner, RecordingAudioListener, AutoCloseable {
  /**
   * The attempts of one core generation (owner 2026-11-19, [MicrophoneRetry]). The take and the core see one source: its PCM and its end carry the generation
   * of the first source that started (`tap`), a retry is told to nobody.
   */
  private class Run(val core: ULong) {
    var tap: Long? = null
    var attempt = 0
  }

  /** The source of the current attempt of a run; `heard` once its first sound came, no retry after it. */
  private class Live(val run: Run, val audio: Long) { var heard = false }

  private val lock = Any()
  private val microphone = microphone(this)
  private var live: Live? = null
  /** The run waiting the pause before its next attempt. */
  private var pause: Run? = null
  private var clock: AutoCloseable? = null
  /** The last run told SILENCED: a start of a take runs the attempts again ([retry]). */
  private var silent = false
  private var closed = false
  private var input: AudioInputHandle? = null
  private val opened = link.openInput(kind, this)

  val audioGeneration: Long? get() = synchronized(lock) { (live?.run ?: pause)?.tap }

  override suspend fun idle() = microphone.idle()

  override fun restore(input: AudioInputHandle, generation: ULong) {
    synchronized(lock) {
      if (closed) return
      this.input = input
      release()
      attempt(Run(generation))
    }
  }

  /** A start of a take retries a microphone whose last run ended silent (owner 2026-11-19): a fresh run of attempts on the same core generation, told to nobody. */
  fun retry() {
    synchronized(lock) {
      if (closed || !silent) return
      val core = live?.run?.core ?: return
      release()
      attempt(Run(core))
    }
  }

  override fun stop(generation: ULong) {
    synchronized(lock) {
      if ((live?.run ?: pause)?.core != generation) return
      release()
    }
  }

  override fun onLive(generation: Long) = whileLive(generation) { handle, current ->
    heard(current)
    silent = false
    issue(CaptureAudioIssue.ACTIVE)
    handle.sound(current.run.core, true)
  }

  override fun onSilenced(generation: Long, silenced: Boolean) = whileLive(generation) { handle, current ->
    if (silenced && retry(current, stopMicrophone = true)) return@whileLive
    if (!silenced) heard(current)
    silent = silenced
    issue(if (silenced) CaptureAudioIssue.SILENCED else CaptureAudioIssue.ACTIVE)
    handle.sound(current.run.core, !silenced)
  }

  override fun onFailed(generation: Long, failure: NativeFailure) = whileLive(generation) { handle, current ->
    if (retry(current, stopMicrophone = false)) return@whileLive
    release(stopMicrophone = false)
    failed(failure)
    issue(CaptureAudioIssue.SOURCE_ERROR)
    handle.restoreFailed(current.run.core)
  }

  override fun onEnded(generation: Long) = whileLive(generation) { _, _ -> release(stopMicrophone = false) }

  override fun onTimebase(generation: Long, originNanos: Long) = whileLive(generation) { _, current -> tap.onTimebase(current.run.tap!!, originNanos) }

  override fun onPcm(chunk: PcmChunk) = whileLive(chunk.generation) { _, current ->
    val tapped = current.run.tap!!
    tap.onPcm(if (chunk.generation == tapped) chunk else PcmChunk(tapped, chunk.spec, chunk.data, chunk.frames, chunk.firstFrame, chunk.ptsUs))
  }

  /** Starts the next attempt of [run] and its wait for the first sound. Runs under the lock. */
  private fun attempt(run: Run) {
    val handle = input ?: return
    run.attempt += 1
    val audio = try { microphone.start(spec) } catch (error: Exception) {
      if (run.attempt < MicrophoneRetry.ATTEMPTS) return wait(run)
      end(run)
      failed(NativeFailure(FailureSource.RECORDER, error.javaClass.name, 0L, error.message ?: error.toString()))
      issue(CaptureAudioIssue.SOURCE_ERROR)
      handle.restoreFailed(run.core)
      connected()
      return
    }
    if (run.tap == null) run.tap = audio
    val current = Live(run, audio)
    live = current
    connected()
    clock = timer.schedule(MicrophoneRetry.SOUND_WAIT_MS) {
      synchronized(lock) {
        if (live !== current || current.heard) return@synchronized
        clock = null
        if (retry(current, stopMicrophone = true)) return@synchronized
        // The last attempt gave no sound in time: the same word as a source the system silenced.
        silent = true
        issue(CaptureAudioIssue.SILENCED)
        input?.sound(run.core, false)
      }
    }
  }

  /** The source of [current] failed before its first sound: true when another attempt follows after the pause. Runs under the lock. */
  private fun retry(current: Live, stopMicrophone: Boolean): Boolean {
    if (current.heard || current.run.attempt >= MicrophoneRetry.ATTEMPTS) return false
    clock?.close()
    clock = null
    live = null
    if (stopMicrophone) microphone.stop(current.audio)
    wait(current.run)
    return true
  }

  private fun wait(run: Run) {
    pause = run
    clock = timer.schedule(MicrophoneRetry.PAUSE_MS) {
      synchronized(lock) {
        if (closed || pause !== run) return@synchronized
        pause = null
        clock = null
        attempt(run)
      }
    }
  }

  private fun heard(current: Live) {
    current.heard = true
    clock?.close()
    clock = null
  }

  /** The one place a run of the source ends: the take hears it once, whoever ended it. Runs under the lock. */
  private fun release(stopMicrophone: Boolean = true) {
    silent = false
    clock?.close()
    clock = null
    val waiting = pause
    pause = null
    val current = live
    live = null
    if (current != null && stopMicrophone) microphone.stop(current.audio)
    (current?.run ?: waiting)?.let(::end)
  }

  private fun end(run: Run) { run.tap?.let(tap::onSourceEnd) }

  // Every word of the microphone runs under the lock that stop and close take: nothing reaches the core or the take after its generation ended.
  private fun whileLive(generation: Long, deliver: (AudioInputHandle, Live) -> Unit) {
    synchronized(lock) {
      val current = live?.takeIf { it.audio == generation } ?: return
      deliver(input ?: return, current)
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
