package com.noma4i.medialayer.ui

import android.media.AudioFormat
import com.noma4i.medialayer.FailureSource
import com.noma4i.medialayer.NativeFailure
import com.noma4i.medialayer.VoiceEncoderSettings
import java.io.File

internal interface VoiceTakeListener {
  fun onStarted()

  fun onLimit()

  fun onFinished(file: File, durationUs: Long)

  fun onFailed(failure: NativeFailure)

  fun onCancelled()

  fun onLost()
}

// One lock owns the phase and every writer call under it is non-blocking; events leave in transition order, outside the lock, and every transition after the end is dropped.
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
  private val events = ArrayDeque<(VoiceTakeListener) -> Unit>()
  private var delivering = false
  private var state = State.WAITING
  private var pending: ((VoiceTakeListener) -> Unit)? = null
  private var limited = false
  private var frames = 0L

  fun pcm(chunk: PcmChunk) = transition {
    if (chunk.generation != generation || (state != State.WAITING && state != State.OPEN)) return@transition
    val first = state == State.WAITING
    try {
      require(matches(chunk)) { "PCM does not match the voice encoder" }
      if (first) writer.open(file)
      state = State.OPEN
      writer.append(chunk)
    } catch (error: Throwable) {
      return@transition end { it.onFailed(voiceFailure(error)) }
    }
    if (first) events += { it.onStarted() }
    frames += chunk.frames
    if (!limited && frames * 1_000_000L / settings.sampleRateHz.toLong() >= limitUs) { limited = true; events += { it.onLimit() } }
  }

  fun finish() = transition {
    when (state) {
      State.WAITING -> end { it.onCancelled() }
      State.OPEN -> { state = State.FINISHING; writer.finish(::complete) }
      else -> Unit
    }
  }

  fun cancel() = transition { if (state == State.FINISHING) pending = pending ?: { it.onCancelled() } else end { it.onCancelled() } }

  fun lost() = transition { if (state == State.FINISHING) pending = pending ?: { it.onLost() } else end { it.onLost() } }

  fun fail(error: Throwable) = fail(voiceFailure(error))

  fun fail(failure: NativeFailure) = transition { if (state == State.FINISHING) pending = pending ?: { it.onFailed(failure) } else end { it.onFailed(failure) } }

  private fun complete(result: Result<Long>) = transition {
    if (state != State.FINISHING) return@transition
    val done = result.getOrNull()
    val failed: ((VoiceTakeListener) -> Unit)? = result.exceptionOrNull()?.let { error -> { it.onFailed(voiceFailure(error)) } }
    val outcome = pending ?: failed
    state = State.ENDED
    if (outcome == null) events += { it.onFinished(file, done!!) } else { file.delete(); events += outcome }
  }

  private fun end(outcome: (VoiceTakeListener) -> Unit) {
    if (state == State.ENDED) return
    writer.cancel()
    state = State.ENDED
    file.delete()
    events += outcome
  }

  private fun transition(change: () -> Unit) {
    if (Thread.holdsLock(lock)) return change()
    synchronized(lock) { change(); if (delivering) return else delivering = true }
    while (true) {
      val next = synchronized(lock) { events.removeFirstOrNull() ?: run { delivering = false; return } }
      next(listener)
    }
  }

  private fun matches(chunk: PcmChunk): Boolean {
    val channels = Integer.bitCount(chunk.spec.channelMask)
    return chunk.spec.encoding == AudioFormat.ENCODING_PCM_16BIT && chunk.spec.sampleRate.toLong() == settings.sampleRateHz.toLong() &&
      channels.toLong() == settings.channels.toLong() && chunk.frames > 0 && chunk.data.remaining() == chunk.frames * channels * 2
  }
}

internal fun voiceFailure(error: Throwable) = NativeFailure(FailureSource.RECORDER, "android.media.MediaCodec", 0, error.message ?: error.javaClass.name)
