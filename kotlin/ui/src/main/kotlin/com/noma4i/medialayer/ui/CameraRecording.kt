package com.noma4i.medialayer.ui

import android.view.Surface
import com.noma4i.medialayer.AudioCoverage
import com.noma4i.medialayer.CaptureEvent
import com.noma4i.medialayer.FailureSource
import com.noma4i.medialayer.NativeFailure
import com.noma4i.medialayer.RecordEnd
import java.io.File
import java.util.concurrent.Executor
import java.util.concurrent.atomic.AtomicBoolean

internal interface CameraTaking {
  fun begin(): Boolean

  fun audio(chunk: PcmChunk)

  fun audioTimebase(generation: Long, originNanos: Long)

  fun confirm()

  fun finish()

  /** The audio source is gone: a take that waits for its PCM ends with what it has. */
  fun audioEnded() = Unit

  fun cancel()
}

internal interface CameraEncoderFeed {
  fun attach(generation: Long, target: Surface)

  fun detach(generation: Long)

  fun encoded(generation: Long, ptsUs: Long)
}

/** A take records only after its own first PCM and the confirm of the gate; a take that ends before it started, or fails, gives one `RecordFinished(FAILED)`. Every callback of a take that is not the current one is dropped. */
internal class CameraRecording(
  private val main: Executor,
  private val emit: (CaptureEvent) -> Unit,
  private val judged: (AudioCoverage) -> Unit = {},
  private val make: (Long, File, CameraTakeSettings, CameraTakeListener) -> CameraTaking
) : CameraAudioTap {
  private class Take(val generation: Long, val audio: Long?, val feed: CameraEncoderFeed, val gate: (() -> Unit) -> Unit, val stale: () -> Boolean, val finalized: () -> Unit) {
    var take: CameraTaking? = null
    val firstPcm = AtomicBoolean()
    @Volatile var baseUs = -1L
    var gated = false
    var started = false
    var stopping = false
  }

  @Volatile private var current: Take? = null
  @Volatile private var origin: Pair<Long, Long>? = null
  @Volatile private var recordedNanos = 0L
  private val stopCallbacks = mutableListOf<() -> Unit>()

  val active: Boolean get() = current != null

  fun recordedMs(): Long = recordedNanos / 1_000_000

  fun start(file: File, settings: CameraTakeSettings, feed: CameraEncoderFeed, gate: (() -> Unit) -> Unit, stale: () -> Boolean, generation: Long, finalized: () -> Unit) {
    recordedNanos = 0
    val entry = Take(generation, settings.audio?.let { settings.audioGeneration }, feed, gate, stale, finalized)
    try {
      val made = make(generation, file, settings, listener(entry)).also { entry.take = it }
      current = entry
      origin?.takeIf { it.first == entry.audio }?.let { made.audioTimebase(it.first, it.second) }
      check(made.begin()) { "the take has no thread" }
    } catch (error: Exception) {
      current = null
      entry.take?.cancel()
      Media.recordNativeFailure(FailureSource.RECORDER, error)
      emit(CaptureEvent.RecordFinished(RecordEnd.FAILED))
    }
  }

  private fun listener(entry: Take) = object : CameraTakeListener {
    override fun onBegan(generation: Long, inputSurface: Surface) = main.execute {
      if (current !== entry) return@execute
      entry.feed.attach(generation, inputSurface)
      if (entry.audio == null) open(entry)
    }

    override fun onVideoEncoded(generation: Long, ptsUs: Long) = main.execute {
      if (current !== entry || entry.stale()) return@execute
      entry.feed.encoded(generation, ptsUs)
      if (entry.baseUs >= 0) recordedNanos = maxOf(0L, ptsUs - entry.baseUs) * 1000
    }

    override fun onStarted(generation: Long, baseUs: Long) {
      entry.baseUs = baseUs
      main.execute {
        if (current !== entry) return@execute
        entry.started = true
        if (!entry.stale()) emit(CaptureEvent.RecordStarted)
      }
    }

    override fun onFinished(generation: Long, file: File, durationUs: Long, coverage: AudioCoverage) = main.execute {
      if (current === entry && !entry.stale()) judged(coverage)
      settle(entry, if (entry.stopping) RecordEnd.COMPLETED else RecordEnd.LIMIT_REACHED)
    }

    override fun onFailed(generation: Long, failure: NativeFailure) = main.execute {
      if (current === entry && !entry.stale()) Media.recordNativeFailure(failure)
      settle(entry, RecordEnd.FAILED)
    }

    override fun onCancelled(generation: Long) = main.execute { settle(entry, RecordEnd.FAILED) }
  }

  private fun open(entry: Take) {
    if (current !== entry || entry.gated || entry.stale()) return
    entry.gated = true
    entry.gate { if (current === entry) entry.take?.confirm() }
  }

  private fun settle(entry: Take, end: RecordEnd) {
    if (current !== entry) return
    current = null
    entry.feed.detach(entry.generation)
    if (entry.stale()) return completeStops()
    emit(CaptureEvent.RecordFinished(end))
    completeStops()
    entry.finalized()
  }

  fun encoderFailed(generation: Long, failure: NativeFailure) = main.execute {
    val entry = current?.takeIf { it.generation == generation } ?: return@execute
    if (!entry.stale()) Media.recordNativeFailure(failure)
    entry.take?.cancel()
  }

  override fun onTimebase(generation: Long, originNanos: Long) {
    origin = generation to originNanos
    current?.takeIf { it.audio == generation }?.take?.audioTimebase(generation, originNanos)
  }

  override fun onPcm(chunk: PcmChunk) {
    val entry = current?.takeIf { it.audio == chunk.generation } ?: return
    entry.take?.audio(chunk)
    if (entry.firstPcm.compareAndSet(false, true)) main.execute {
      if (current !== entry || entry.stale()) return@execute
      emit(CaptureEvent.AudioFlow(true))
      open(entry)
    }
  }

  override fun onSourceEnd(generation: Long) { current?.takeIf { it.audio == generation }?.take?.audioEnded() }

  fun audioLost() { current?.take?.audioEnded() }

  fun stop() { current?.let(::end) }

  fun stop(done: () -> Unit) {
    val entry = current
    if (entry == null) done() else {
      stopCallbacks.add(done)
      end(entry)
    }
  }

  fun awaitStopped(done: () -> Unit) { if (current == null) done() else stopCallbacks.add(done) }

  fun close() {
    current?.let {
      current = null
      it.feed.detach(it.generation)
      it.take?.cancel()
    }
    completeStops()
  }

  private fun end(entry: Take) {
    if (entry.started) {
      entry.stopping = true
      entry.take?.finish()
    } else entry.take?.cancel()
  }

  private fun completeStops() {
    val callbacks = stopCallbacks.toList()
    stopCallbacks.clear()
    callbacks.forEach { it() }
  }
}
