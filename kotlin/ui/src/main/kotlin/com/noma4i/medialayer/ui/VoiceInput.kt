package com.noma4i.medialayer.ui

import com.noma4i.medialayer.NativeFailure
import com.noma4i.medialayer.RecordingKind
import com.noma4i.medialayer.VoiceEncoderSettings
import java.io.File
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicInteger

// One phase owner: PREPARING -> WAITING (owner open, no take) -> TAKING -> LOST (told once, owner lingers until the next healthy PCM) -> CLOSED; every transition and the owner's registration happen under one lock, every call out of it.
internal class VoiceInput(
  private val link: AudioLink,
  private val spec: PcmSpec,
  private val settings: VoiceEncoderSettings,
  private val limitUs: Long,
  private val file: File,
  private val writer: VoiceFileWriter,
  private val listener: VoiceTakeListener,
  private val microphone: (RecordingAudioListener) -> CameraMicrophone
) : CameraAudioTap, AutoCloseable {
  private enum class Phase { PREPARING, WAITING, TAKING, LOST, CLOSED }

  private val lock = Any()
  private val peakBits = AtomicInteger()
  private var phase = Phase.PREPARING
  private var owner: CameraAudioInput? = null
  private var take: VoiceTake? = null
  private var generation = -1L

  private val out = object : VoiceTakeListener {
    override fun onStarted() = listener.onStarted()
    override fun onLimit() = listener.onLimit()
    override fun onFinished(file: File, durationUs: Long) { if (leave(Phase.CLOSED, Phase.TAKING)) { release(); listener.onFinished(file, durationUs) } }
    override fun onFailed(failure: NativeFailure) { if (leave(Phase.CLOSED, Phase.TAKING)) { release(); listener.onFailed(failure) } }
    override fun onCancelled() { if (leave(Phase.CLOSED, Phase.TAKING)) { release(); listener.onCancelled() } }
    override fun onLost() { if (leave(Phase.LOST, Phase.TAKING)) listener.onLost() }
  }

  /** The linear peak 0 to 1 of the take's PCM since the last call. */
  fun peak(): Float = peakBits.getAndSet(0) / 32768f

  fun start() {
    try { writer.prepare(settings, ::prepared, { fail(voiceFailure(it)) }) } catch (error: Throwable) { fail(voiceFailure(error)) }
  }

  fun finish() {
    val current = synchronized(lock) { if (phase == Phase.TAKING) take else null }
    if (current != null) current.finish() else cancel()
  }

  fun cancel() {
    route({ endWaiting { it.onCancelled() } }, { it.cancel() }, { leave(Phase.CLOSED, Phase.LOST) })
    release()
  }

  override fun close() = cancel()

  private fun prepared(result: Result<Unit>) {
    result.exceptionOrNull()?.let { return fail(voiceFailure(it)) }
    synchronized(lock) {
      if (phase != Phase.PREPARING) return
      owner = CameraAudioInput(link, spec, this, ::issue, RecordingKind.VOICE, ::fail, microphone)
      phase = Phase.WAITING
    }
  }

  private fun issue(issue: CaptureAudioIssue) {
    if (issue == CaptureAudioIssue.SILENCED) route({ leave(Phase.LOST, Phase.WAITING).also { if (it) { writer.cancel(); listener.onLost() } } }, { it.lost() })
  }

  private fun fail(failure: NativeFailure) {
    route({ endWaiting { it.onFailed(failure) } }, { it.fail(failure) }, { if (leave(Phase.CLOSED, Phase.LOST)) release() })
  }

  override fun onTimebase(generation: Long, originNanos: Long) = Unit

  override fun onPcm(chunk: PcmChunk) {
    var healthyAfterLoss = false
    val current = synchronized(lock) {
      when (phase) {
        Phase.WAITING -> VoiceTake(chunk.generation, file, settings, limitUs, writer, out).also { take = it; generation = chunk.generation; phase = Phase.TAKING }
        Phase.TAKING -> take.takeIf { chunk.generation == generation }
        Phase.LOST -> null.also { healthyAfterLoss = true; phase = Phase.CLOSED }
        else -> null
      }
    }
    if (healthyAfterLoss) release()
    current?.let { record(chunk); it.pcm(chunk) }
  }

  private fun record(chunk: PcmChunk) {
    val samples = chunk.data.duplicate().order(ByteOrder.LITTLE_ENDIAN).asShortBuffer()
    var peak = 0
    while (samples.hasRemaining()) peak = maxOf(peak, Math.abs(samples.get().toInt()))
    peakBits.accumulateAndGet(peak, ::maxOf)
  }

  private fun route(waiting: () -> Boolean, taking: (VoiceTake) -> Unit, lost: () -> Unit = {}) {
    while (true) {
      val (now, current) = synchronized(lock) { phase to take }
      when (now) {
        Phase.PREPARING, Phase.WAITING -> if (waiting()) return
        Phase.TAKING -> return taking(current!!)
        Phase.LOST -> return lost()
        Phase.CLOSED -> return
      }
    }
  }

  private fun endWaiting(event: (VoiceTakeListener) -> Unit): Boolean {
    if (!leave(Phase.CLOSED, Phase.PREPARING, Phase.WAITING)) return false
    writer.cancel()
    release()
    event(listener)
    return true
  }

  private fun leave(to: Phase, vararg from: Phase): Boolean = synchronized(lock) { (phase in from).also { if (it) phase = to } }

  private fun release() {
    val last = synchronized(lock) { owner.also { owner = null } }
    last?.close()
  }
}
