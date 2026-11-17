package com.noma4i.medialayer.ui

import com.noma4i.medialayer.NativeFailure
import com.noma4i.medialayer.RecordingKind
import com.noma4i.medialayer.VoiceEncoderSettings
import java.io.File
import java.nio.ByteOrder
import java.util.concurrent.atomic.AtomicInteger

// One voice take over the input owner of the audio core: the codec is prepared without a file, the owner starts the microphone only after it is ready, and the take opens on the first live PCM of one generation.
// A lost take is told once and the owner stays for the restore the core allows: the next healthy PCM of a later generation closes it and starts no take. Every other end closes the owner; the failure leaves once through `VoiceTakeListener.onFailed(NativeFailure)`: its owner records it.
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
  private val lock = Any()
  private val peakBits = AtomicInteger()
  private var owner: CameraAudioInput? = null
  @Volatile private var take: VoiceTake? = null
  private var generation = -1L
  private var ended = false
  private var released = false

  private val out = object : VoiceTakeListener {
    override fun onStarted() = listener.onStarted()
    override fun onLimit() = listener.onLimit()
    override fun onFinished(file: File, durationUs: Long) = end(true) { listener.onFinished(file, durationUs) }
    override fun onFailed(failure: NativeFailure) = end(true) { listener.onFailed(failure) }
    override fun onCancelled() = end(true) { listener.onCancelled() }
    override fun onLost() = end(false) { listener.onLost() }
  }

  /** The linear peak 0 to 1 of the take's PCM since the last call. */
  fun peak(): Float = peakBits.getAndSet(0) / 32768f

  fun start() {
    try { writer.prepare(settings, ::prepared, ::writerFailed) } catch (error: Throwable) { withoutTake { it.onFailed(voiceFailure(error)) } }
  }

  fun finish() { take?.finish() ?: cancel() }

  fun cancel() {
    take?.cancel() ?: withoutTake { it.onCancelled() }
    release()
  }

  override fun close() = cancel()

  private fun prepared(result: Result<Unit>) {
    result.exceptionOrNull()?.let { return withoutTake { out -> out.onFailed(voiceFailure(it)) } }
    if (synchronized(lock) { ended }) return
    val made = CameraAudioInput(link, spec, this, ::issue, RecordingKind.VOICE, ::sourceFailed, microphone)
    val stale = synchronized(lock) { if (released) true else { owner = made; false } }
    if (stale) made.close()
  }

  private fun issue(issue: CaptureAudioIssue) { if (issue == CaptureAudioIssue.SILENCED) take?.lost() }

  private fun writerFailed(error: Throwable) = sourceFailed(voiceFailure(error))

  private fun sourceFailed(failure: NativeFailure) { take?.fail(failure) ?: withoutTake { it.onFailed(failure) } }

  override fun onTimebase(generation: Long, originNanos: Long) = Unit

  override fun onPcm(chunk: PcmChunk) {
    val (current, later) = synchronized(lock) {
      when {
        take == null && !ended -> VoiceTake(chunk.generation, file, settings, limitUs, writer, out).also { take = it; generation = chunk.generation } to false
        chunk.generation == generation -> take to false
        else -> null to ended
      }
    }
    if (later) release()
    current?.let { record(chunk); it.pcm(chunk) }
  }

  private fun record(chunk: PcmChunk) {
    val samples = chunk.data.duplicate().order(ByteOrder.LITTLE_ENDIAN).asShortBuffer()
    var peak = 0
    while (samples.hasRemaining()) peak = maxOf(peak, Math.abs(samples.get().toInt()))
    peakBits.accumulateAndGet(peak, ::maxOf)
  }

  // An end without a take holds the prepared codec: it is released with the end, never left to the owner's life.
  private fun withoutTake(event: (VoiceTakeListener) -> Unit) {
    if (take != null) return
    writer.cancel()
    event(out)
  }

  private fun end(release: Boolean, deliver: () -> Unit) {
    synchronized(lock) { if (ended) return; ended = true }
    if (release) release()
    deliver()
  }

  private fun release() {
    val last = synchronized(lock) { if (released) null else { released = true; owner.also { owner = null } } }
    last?.close()
  }
}
