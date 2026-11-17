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
  fun open(file: File, settings: VoiceEncoderSettings)

  fun append(chunk: PcmChunk)

  fun finish(done: (Result<Long>) -> Unit)

  fun cancel()
}

// Nothing here waits: append hands the PCM to a free codec input or throws, the codec callbacks run on one thread, `done` and the take run outside `gate`.
internal class MediaCodecVoiceFileWriter : VoiceFileWriter {
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
  private var done: ((Result<Long>) -> Unit)? = null
  private var written = 0
  private var rate = 0
  private var baseUs = -1L
  private var endUs = 0L

  override fun open(file: File, settings: VoiceEncoderSettings) = synchronized(gate) {
    try {
      rate = settings.sampleRateHz.toInt()
      val format = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_AAC, rate, settings.channels.toInt()).apply {
        setInteger(MediaFormat.KEY_AAC_PROFILE, MediaCodecInfo.CodecProfileLevel.AACObjectLC)
        setInteger(MediaFormat.KEY_BIT_RATE, settings.bitRate.toInt())
      }
      thread.start()
      codec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_AUDIO_AAC).also {
        it.setCallback(callback, Handler(thread.looper))
        it.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        it.start()
      }
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
      (closed).also { if (!it) { this.done = done; eosPending = true; queueEnd() } }
    }
    if (rejected) done(Result.failure(failure ?: IllegalStateException("the writer is closed")))
  }

  override fun cancel() = synchronized(gate) {
    done = null
    closeLocked()
  }

  private fun queueEnd() {
    if (!eosPending || free.isEmpty()) return
    eosPending = false
    codec!!.queueInputBuffer(free.removeFirst(), 0, 0, endUs, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
  }

  private fun conclude(error: Throwable?) {
    var result: Result<Long>? = null
    val callback = synchronized(gate) {
      if (closed) return
      if (error != null) failure = error
      result = if (error != null) Result.failure(error) else runCatching { check(written > 0) { "the take holds no audio" }; muxer!!.stop(); muxing = false; endUs }
      done.also { done = null; closeLocked() }
    }
    callback?.invoke(result!!)
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
    override fun onInputBufferAvailable(codec: MediaCodec, index: Int) = synchronized(gate) {
      if (closed) return@synchronized
      free.addLast(index)
      try { queueEnd() } catch (error: Throwable) { failure = error }
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
      synchronized(gate) {
        if (closed) return
        try { track = muxer!!.addTrack(format); muxer!!.start(); muxing = true } catch (thrown: Throwable) { failure = thrown }
      }
    }

    override fun onError(codec: MediaCodec, error: MediaCodec.CodecException) = conclude(error)
  }
}
