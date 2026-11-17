package com.noma4i.medialayer.ui

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.media.MediaMuxer
import android.os.Handler
import android.os.HandlerThread
import com.noma4i.medialayer.VoiceEncoderSettings
import java.io.File

internal interface VoiceFileWriter {
  fun prepare(settings: VoiceEncoderSettings, ready: (Result<Unit>) -> Unit, failed: (Throwable) -> Unit)

  fun open(file: File)

  fun append(chunk: PcmChunk)

  fun finish(done: (Result<Long>) -> Unit)

  fun cancel()
}

// Nothing here waits; the codec is prepared without a file, a native error ends in `conclude` and reaches the owner once: by `done`, else `ready`, else `failed`.
internal class MediaCodecVoiceFileWriter(private val inputFree: () -> Unit = {}) : VoiceFileWriter {
  private val gate = Any()
  private val thread = HandlerThread("VoiceFileWriter")
  private val free = ArrayDeque<Int>()
  private var codec: MediaCodec? = null
  private var muxer: MediaMuxer? = null
  private var track = -1
  private var muxing = false
  private var closed = false
  private var failure: Throwable? = null
  private var eosPending = false
  private var ready: ((Result<Unit>) -> Unit)? = null
  private var failed: ((Throwable) -> Unit)? = null
  private var done: ((Result<Long>) -> Unit)? = null
  private var written = 0
  private var rate = 0
  private var baseUs = -1L
  private var endUs = 0L

  override fun prepare(settings: VoiceEncoderSettings, ready: (Result<Unit>) -> Unit, failed: (Throwable) -> Unit) = synchronized(gate) {
    check(!closed && codec == null) { "the writer is already used" }
    try {
      rate = settings.sampleRateHz.toInt()
      val format = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_AAC, rate, settings.channels.toInt()).apply {
        setInteger(MediaFormat.KEY_AAC_PROFILE, MediaCodecInfo.CodecProfileLevel.AACObjectLC)
        setInteger(MediaFormat.KEY_BIT_RATE, settings.bitRate.toInt())
      }
      this.ready = ready
      this.failed = failed
      thread.start()
      codec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_AUDIO_AAC).also {
        it.setCallback(callback, Handler(thread.looper))
        it.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        it.start()
      }
    } catch (error: Throwable) {
      closeLocked()
      throw error
    }
  }

  override fun open(file: File) = synchronized(gate) {
    check(!closed && codec != null && muxer == null) { failure?.message ?: "the writer is not prepared" }
    try {
      muxer = MediaMuxer(file.path, MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4)
    } catch (error: Throwable) {
      closeLocked()
      throw error
    }
  }

  override fun append(chunk: PcmChunk) = synchronized(gate) {
    check(!closed) { failure?.message ?: "the writer is closed" }
    val encoder = codec!!
    val source = chunk.data.duplicate()
    val bytesPerFrame = chunk.data.remaining() / chunk.frames
    if (baseUs < 0) baseUs = chunk.ptsUs
    var consumed = 0
    while (source.hasRemaining()) {
      check(free.isNotEmpty()) { "the encoder has no free input" }
      val index = free.removeFirst()
      val input = encoder.getInputBuffer(index)!!
      val size = minOf(input.remaining(), source.remaining()).let { it - it % bytesPerFrame }
      check(size > 0) { "audio input buffer holds no whole frame" }
      input.put(source.duplicate().apply { limit(position() + size) })
      source.position(source.position() + size)
      val ptsUs = chunk.ptsUs - baseUs + consumed * 1_000_000L / rate
      consumed += size / bytesPerFrame
      endUs = ptsUs + size / bytesPerFrame * 1_000_000L / rate
      encoder.queueInputBuffer(index, 0, size, ptsUs, 0)
    }
  }

  override fun finish(done: (Result<Long>) -> Unit) {
    val rejected = synchronized(gate) {
      closed.also { if (!it) { this.done = done; eosPending = true } }
    }
    if (rejected) return done(Result.failure(failure ?: IllegalStateException("the writer is closed")))
    queueEnd()
  }

  override fun cancel() = synchronized(gate) {
    done = null
    ready = null
    failed = null
    closeLocked()
  }

  private fun queueEnd() {
    val error = synchronized(gate) {
      if (closed || !eosPending || free.isEmpty()) return
      eosPending = false
      try { codec!!.queueInputBuffer(free.removeFirst(), 0, 0, endUs, MediaCodec.BUFFER_FLAG_END_OF_STREAM); null } catch (thrown: Throwable) { thrown }
    }
    if (error != null) conclude(error)
  }

  private fun conclude(error: Throwable?) {
    val finishing: ((Result<Long>) -> Unit)?
    val starting: ((Result<Unit>) -> Unit)?
    val pushing: ((Throwable) -> Unit)?
    val result: Result<Long>
    synchronized(gate) {
      if (closed) return
      result = if (error != null) Result.failure(error) else runCatching { check(written > 0) { "the take holds no audio" }; muxer!!.stop(); muxing = false; endUs }
      failure = result.exceptionOrNull()
      finishing = done
      starting = ready
      pushing = failed
      done = null
      ready = null
      failed = null
      closeLocked()
    }
    finishing?.invoke(result)
    result.exceptionOrNull()?.let { if (starting != null) starting.invoke(Result.failure(it)) else if (finishing == null) pushing?.invoke(it) }
  }

  private fun closeLocked() {
    closed = true
    free.clear()
    val encoder = codec
    val container = muxer
    val wasMuxing = muxing
    codec = null
    muxer = null
    muxing = false
    if (!thread.isAlive) return
    Handler(thread.looper).post {
      try { encoder?.stop() } catch (_: Throwable) {}
      try { encoder?.release() } catch (_: Throwable) {}
      try { if (wasMuxing) container?.stop() } catch (_: Throwable) {}
      try { container?.release() } catch (_: Throwable) {}
      thread.quitSafely()
    }
  }

  private val callback = object : MediaCodec.Callback() {
    override fun onInputBufferAvailable(codec: MediaCodec, index: Int) {
      val announce: ((Result<Unit>) -> Unit)?
      synchronized(gate) {
        if (closed) return
        free.addLast(index)
        announce = ready
        ready = null
      }
      announce?.invoke(Result.success(Unit))
      queueEnd()
      inputFree()
    }

    override fun onOutputBufferAvailable(codec: MediaCodec, index: Int, info: MediaCodec.BufferInfo) {
      var end = false
      var error: Throwable? = null
      synchronized(gate) {
        if (closed) return
        try {
          if (info.size > 0 && info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG == 0) {
            val output = codec.getOutputBuffer(index)!!.apply { position(info.offset); limit(info.offset + info.size) }
            muxer!!.writeSampleData(track, output, info)
            written++
          }
          codec.releaseOutputBuffer(index, false)
          end = info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0
        } catch (thrown: Throwable) {
          error = thrown
        }
      }
      if (end || error != null) conclude(error)
    }

    override fun onOutputFormatChanged(codec: MediaCodec, format: MediaFormat) {
      val error = synchronized(gate) {
        if (closed) return
        try { track = muxer!!.addTrack(format); muxer!!.start(); muxing = true; null } catch (thrown: Throwable) { thrown }
      }
      if (error != null) conclude(error)
    }

    override fun onError(codec: MediaCodec, error: MediaCodec.CodecException) = conclude(error)
  }
}
