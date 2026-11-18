package com.noma4i.medialayer.ui

import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.AudioRecordingConfiguration
import android.media.AudioTimestamp
import com.noma4i.medialayer.FailureSource
import com.noma4i.medialayer.NativeFailure
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.update
import java.nio.ByteBuffer
import java.util.concurrent.Executor
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

internal data class PcmSpec(
  val audioSource: Int,
  val sampleRate: Int,
  val channelMask: Int,
  val encoding: Int,
  val bufferBytes: Int? = null,
  val chunkBytes: Int? = null
)

// data is reused by the next read: a retainer that does not copy inside onPcm sees overwritten PCM.
internal class PcmChunk(
  val generation: Long,
  val spec: PcmSpec,
  val data: ByteBuffer,
  val frames: Int,
  val firstFrame: Long,
  val ptsUs: Long
)

internal interface RecordingAudioListener {
  fun onLive(generation: Long)

  fun onTimebase(generation: Long, originNanos: Long)

  fun onPcm(chunk: PcmChunk)

  fun onSilenced(generation: Long, silenced: Boolean)

  fun onFailed(generation: Long, failure: NativeFailure)

  /** The source ended on its own with no fault: the session it served is closed. */
  fun onEnded(generation: Long) = Unit
}

internal class PcmSizeError(val code: Long, message: String) : Exception(message)

/** The buffer and the read chunk of a source in bytes, the chunk aligned to whole frames: what the source leases while it runs. */
internal class PcmSizes(val bytesPerFrame: Int, val bufferBytes: Int, val chunkBytes: Int) {
  val lease: Long get() = (bufferBytes + chunkBytes).toLong()
}

internal fun pcmSizes(spec: PcmSpec): PcmSizes {
  val sample = when (spec.encoding) {
    AudioFormat.ENCODING_PCM_8BIT -> 1
    AudioFormat.ENCODING_PCM_16BIT -> 2
    AudioFormat.ENCODING_PCM_FLOAT -> 4
    else -> throw PcmSizeError(spec.encoding.toLong(), "unsupported encoding ${spec.encoding}")
  }
  val channels = Integer.bitCount(spec.channelMask)
  if (channels <= 0 || spec.sampleRate <= 0) throw PcmSizeError(0, "bad channel mask ${spec.channelMask} or rate ${spec.sampleRate}")
  val bytesPerFrame = sample * channels
  val minimum = AudioRecord.getMinBufferSize(spec.sampleRate, spec.channelMask, spec.encoding)
  if (minimum <= 0) throw PcmSizeError(minimum.toLong(), "AudioRecord.getMinBufferSize returned $minimum")
  val bufferBytes = spec.bufferBytes ?: minimum
  val chunkBytes = (spec.chunkBytes ?: minimum).let { it - it % bytesPerFrame }
  if (chunkBytes <= 0 || bufferBytes <= 0) throw PcmSizeError(0, "no whole frame fits the buffer")
  return PcmSizes(bytesPerFrame, bufferBytes, chunkBytes)
}

internal class RecordingAudio(
  private val listener: RecordingAudioListener,
  private val admit: (Long) -> AutoCloseable,
  private val callbackExecutor: Executor
) : AutoCloseable {
  private val generations = AtomicLong()
  private val lock = Any()
  private var current: Source? = null
  private var closed = false
  private val sources = kotlinx.coroutines.flow.MutableStateFlow(0)
  private val hardware = Executors.newSingleThreadExecutor { Thread(it, "RecordingAudio-hardware").apply { isDaemon = true } }
  private val signals = Executors.newSingleThreadExecutor { Thread(it, "RecordingAudio-signal").apply { isDaemon = true } }

  fun start(spec: PcmSpec): Long {
    val source = Source(generations.incrementAndGet(), spec)
    sources.update { it + 1 }
    val previous = synchronized(lock) {
      check(!closed) { "RecordingAudio is closed" }
      current.also { current = source }.also { hardware.execute(source::run) }
    }
    previous?.stop()
    return source.generation
  }

  override fun close() {
    val last = synchronized(lock) {
      if (closed) return
      closed = true
      current
    }
    last?.stop()
    hardware.execute { signals.shutdown() }
    hardware.shutdown()
  }

  /** Returns once every started source has released its AudioRecord. */
  suspend fun idle() { sources.first { it == 0 } }

  fun stop(generation: Long) {
    synchronized(lock) { current?.takeIf { it.generation == generation } }?.stop()
  }

  private inner class Source(val generation: Long, val spec: PcmSpec) {
    private val open = AtomicBoolean(true)
    private val deliverLock = Any()
    private val resourceLock = Any()
    private var record: AudioRecord? = null
    private var live = false
    private var silenced = false
    private var timebased = false
    private val recordingCallback = object : AudioManager.AudioRecordingCallback() {
      override fun onRecordingConfigChanged(configs: MutableList<AudioRecordingConfiguration>) {
        val config = synchronized(resourceLock) { record?.activeRecordingConfiguration }
        evaluate(config, false)
      }
    }

    fun stop() {
      if (!open.getAndSet(false)) return
      signals.execute { synchronized(resourceLock) { record?.let { try { it.stop() } catch (_: IllegalStateException) {} } } }
    }

    fun run() {
      var reserve: AutoCloseable? = null
      try {
        if (!open.get()) return
        val sizes = try { pcmSizes(spec) } catch (error: PcmSizeError) { return fail(error.code, error.message ?: "bad PCM spec") }
        val bytesPerFrame = sizes.bytesPerFrame
        val bufferBytes = sizes.bufferBytes
        val chunkBytes = sizes.chunkBytes
        reserve = try { admit(sizes.lease) } catch (_: CaptureMemoryClosed) { return end() }
        val made = AudioRecord.Builder()
          .setAudioSource(spec.audioSource)
          .setAudioFormat(AudioFormat.Builder().setSampleRate(spec.sampleRate).setChannelMask(spec.channelMask).setEncoding(spec.encoding).build())
          .setBufferSizeInBytes(bufferBytes)
          .build()
        synchronized(resourceLock) { if (open.get()) record = made }
        if (record == null) return made.release()
        if (made.state != AudioRecord.STATE_INITIALIZED) return fail(made.state.toLong(), "AudioRecord is not initialized")
        made.registerAudioRecordingCallback(callbackExecutor, recordingCallback)
        made.startRecording()
        if (made.recordingState != AudioRecord.RECORDSTATE_RECORDING) return fail(made.recordingState.toLong(), "AudioRecord did not start")
        readLoop(made, chunkBytes, bytesPerFrame)
      } catch (failure: Throwable) {
        fail(0, failure.message ?: failure.javaClass.name)
      } finally {
        release(reserve)
      }
    }

    private fun readLoop(made: AudioRecord, chunkBytes: Int, bytesPerFrame: Int) {
      val buffer = ByteBuffer.allocateDirect(chunkBytes)
      var frames = 0L
      while (open.get()) {
        buffer.clear()
        val read = made.read(buffer, chunkBytes, AudioRecord.READ_BLOCKING)
        if (!open.get()) return
        if (read <= 0) return fail(read.toLong(), "AudioRecord.read returned $read")
        val count = read / bytesPerFrame
        buffer.limit(read)
        val first = frames
        frames += count
        if (!evaluate(made.activeRecordingConfiguration, true)) continue
        checkTimebase(made)
        deliver { listener.onPcm(PcmChunk(generation, spec, buffer, count, first, first * 1_000_000L / spec.sampleRate)) }
      }
    }

    private fun checkTimebase(made: AudioRecord) {
      if (timebased) return
      val stamp = AudioTimestamp()
      if (made.getTimestamp(stamp, AudioTimestamp.TIMEBASE_BOOTTIME) != AudioRecord.SUCCESS) return
      timebased = true
      val origin = stamp.nanoTime - stamp.framePosition * 1_000_000_000L / spec.sampleRate
      deliver { listener.onTimebase(generation, origin) }
    }

    private fun evaluate(config: AudioRecordingConfiguration?, fromRead: Boolean): Boolean = synchronized(deliverLock) {
      if (!open.get()) return false
      if (config == null || config.isClientSilenced) {
        if (live && !silenced) {
          silenced = true
          listener.onSilenced(generation, true)
        }
        return false
      }
      if (!fromRead) return false
      if (!live) {
        live = true
        listener.onLive(generation)
      } else if (silenced) {
        silenced = false
        listener.onSilenced(generation, false)
      }
      true
    }

    private fun end() = synchronized(deliverLock) { if (open.getAndSet(false)) listener.onEnded(generation) }

    private fun deliver(call: () -> Unit) = synchronized(deliverLock) { if (open.get()) call() }

    private fun fail(code: Long, message: String) = synchronized(deliverLock) {
      if (open.getAndSet(false)) listener.onFailed(generation, NativeFailure(FailureSource.RECORDER, "android.media.AudioRecord", code, message))
    }

    private fun release(reserve: AutoCloseable?) {
      open.set(false)
      synchronized(resourceLock) {
        val made = record ?: return@synchronized
        record = null
        try { made.unregisterAudioRecordingCallback(recordingCallback) } catch (_: RuntimeException) {}
        try { made.stop() } catch (_: IllegalStateException) {}
        made.release()
      }
      try { reserve?.close() } finally {
        synchronized(lock) { if (current === this) current = null }
        sources.update { it - 1 }
      }
    }
  }
}
