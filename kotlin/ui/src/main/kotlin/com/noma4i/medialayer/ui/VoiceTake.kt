package com.noma4i.medialayer.ui

import android.media.AudioFormat
import com.noma4i.medialayer.FailureSource
import com.noma4i.medialayer.NativeFailure
import com.noma4i.medialayer.VoiceEncoderSettings
import java.io.File

/** One outcome per take: `onStarted` can come first, then exactly one of `onFinished`, `onFailed`, `onCancelled`, `onLost`. `onLimit` asks for `finish`. */
internal interface VoiceTakeListener {
  fun onStarted()

  fun onLimit()

  fun onFinished(file: File, durationUs: Long)

  fun onFailed(failure: NativeFailure)

  fun onCancelled()

  fun onLost()
}

/**
 * The PCM of one generation into one m4a. No file exists before the first PCM; the first PCM opens the writer. A cancel, a loss and an error leave no file.
 * The take holds no PCM and no lock across a writer call: `finish` returns at once and the result comes when the file is closed.
 */
internal class VoiceTake(
  private val generation: Long,
  private val file: File,
  private val settings: VoiceEncoderSettings,
  private val limitUs: Long,
  private val writer: VoiceFileWriter,
  private val listener: VoiceTakeListener
) {
  private enum class State { WAITING, OPEN, FINISHING, ENDED }

  private val lock = Any()
  private var state = State.WAITING
  private var cancelled = false
  private var limited = false
  private var frames = 0L

  /** Called one at a time by the source. */
  fun pcm(chunk: PcmChunk) {
    if (chunk.generation != generation) return
    val first = synchronized(lock) {
      if (state != State.WAITING && state != State.OPEN) return
      (state == State.WAITING).also { state = State.OPEN }
    }
    try {
      require(matches(chunk)) { "PCM does not match the voice encoder" }
      if (first) writer.open(file, settings)
      writer.append(chunk)
    } catch (error: Throwable) {
      return fail(error)
    }
    if (first) listener.onStarted()
    frames += chunk.frames
    val overLimit = synchronized(lock) { (state == State.OPEN && !limited && frames * 1_000_000L / settings.sampleRateHz.toLong() >= limitUs).also { if (it) limited = true } }
    if (overLimit) listener.onLimit()
  }

  fun finish() {
    val opened = synchronized(lock) {
      if (state == State.OPEN) { state = State.FINISHING; true } else false
    }
    if (opened) writer.finish(::complete) else end(false) { it.onCancelled() }
  }

  fun cancel() = end(true) { it.onCancelled() }

  /** The source is gone before the take finished. */
  fun lost() = end(false) { it.onLost() }

  fun fail(error: Throwable) = fail(NativeFailure(FailureSource.RECORDER, "android.media.MediaCodec", 0, error.message ?: error.javaClass.name))

  fun fail(failure: NativeFailure) = end(false) { it.onFailed(failure) }

  private fun matches(chunk: PcmChunk) =
    chunk.spec.encoding == AudioFormat.ENCODING_PCM_16BIT && chunk.spec.sampleRate.toLong() == settings.sampleRateHz.toLong() &&
      Integer.bitCount(chunk.spec.channelMask).toLong() == settings.channels.toLong() && chunk.frames > 0 &&
      chunk.data.remaining() == chunk.frames * Integer.bitCount(chunk.spec.channelMask) * 2

  private fun complete(result: Result<Long>) {
    val wasCancelled = synchronized(lock) { state = State.ENDED; cancelled }
    val done = result.getOrNull()
    if (wasCancelled || done == null) file.delete()
    when {
      wasCancelled -> listener.onCancelled()
      done != null -> listener.onFinished(file, done)
      else -> listener.onFailed(NativeFailure(FailureSource.RECORDER, "android.media.MediaCodec", 0, result.exceptionOrNull()?.message ?: "the file did not close"))
    }
  }

  /** Ends a take that has not finished; a cancel during the close waits for it, so the closing file is removed after the native completion. */
  private fun end(cancelling: Boolean, report: (VoiceTakeListener) -> Unit) {
    val opened = synchronized(lock) {
      when (state) {
        State.ENDED -> return
        State.FINISHING -> { if (cancelling) cancelled = true; return }
        else -> (state == State.OPEN).also { state = State.ENDED }
      }
    }
    if (opened) writer.cancel()
    file.delete()
    report(listener)
  }
}
