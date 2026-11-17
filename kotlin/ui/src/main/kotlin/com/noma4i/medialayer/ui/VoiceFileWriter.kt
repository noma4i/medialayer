package com.noma4i.medialayer.ui

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.media.MediaMuxer
import com.noma4i.medialayer.VoiceEncoderSettings
import java.io.File

/** The file side of a voice take: the file appears at `open`, which the take calls with the first PCM, and is final only when `finish` reports. */
internal interface VoiceFileWriter {
  /** Throws when the writer is not ready; no file is left. */
  fun open(file: File, settings: VoiceEncoderSettings)

  /** Consumes the chunk before it returns: the chunk data is reused by the next read. */
  fun append(chunk: PcmChunk)

  /** Does not block: the result is the length of the audio in microseconds, once the file is closed. */
  fun finish(done: (Result<Long>) -> Unit)

  /** Stops the writer, a running `append` or `finish` ends with an error the caller ignores. */
  fun cancel()
}

/** AAC in m4a through MediaCodec and MediaMuxer. The codec input is the only buffer: `append` waits for a free input and hands the PCM over. */
internal class MediaCodecVoiceFileWriter : VoiceFileWriter {
  private val lock = Any()
  @Volatile private var closed = false
  private var codec: MediaCodec? = null
  private var muxer: MediaMuxer? = null
  private var track = -1
  private var muxing = false
  private var written = 0
  private var rate = 0
  private var baseUs = -1L
  private var endUs = 0L
  private val info = MediaCodec.BufferInfo()

  override fun open(file: File, settings: VoiceEncoderSettings) = synchronized(lock) {
    try {
      rate = settings.sampleRateHz.toInt()
      val format = MediaFormat.createAudioFormat(MediaFormat.MIMETYPE_AUDIO_AAC, rate, settings.channels.toInt()).apply {
        setInteger(MediaFormat.KEY_AAC_PROFILE, MediaCodecInfo.CodecProfileLevel.AACObjectLC)
        setInteger(MediaFormat.KEY_BIT_RATE, settings.bitRate.toInt())
      }
      codec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_AUDIO_AAC).also {
        it.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        it.start()
      }
      muxer = MediaMuxer(file.path, MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4)
    } catch (error: Throwable) {
      release()
      throw error
    }
  }

  override fun append(chunk: PcmChunk) = synchronized(lock) {
    try {
      check(!closed) { "the writer is closed" }
      val encoder = codec!!
      val source = chunk.data.duplicate()
      val bytesPerFrame = chunk.data.remaining() / chunk.frames
      if (baseUs < 0) baseUs = chunk.ptsUs
      var consumed = 0
      while (source.hasRemaining()) {
        val index = encoder.dequeueInputBuffer(-1)
        val input = encoder.getInputBuffer(index)!!
        val size = minOf(input.remaining(), source.remaining()).let { it - it % bytesPerFrame }
        check(size > 0) { "audio input buffer holds no whole frame" }
        val slice = source.duplicate().apply { limit(position() + size) }
        input.put(slice)
        source.position(source.position() + size)
        val ptsUs = chunk.ptsUs - baseUs + consumed * 1_000_000L / rate
        consumed += size / bytesPerFrame
        endUs = ptsUs + size / bytesPerFrame * 1_000_000L / rate
        encoder.queueInputBuffer(index, 0, size, ptsUs, 0)
        drain(false)
      }
    } catch (error: Throwable) {
      if (closed) return@synchronized
      release()
      throw error
    }
  }

  override fun finish(done: (Result<Long>) -> Unit) {
    Thread({
      done(runCatching {
        synchronized(lock) {
          try {
            check(!closed) { "the writer is closed" }
            val encoder = codec!!
            encoder.queueInputBuffer(encoder.dequeueInputBuffer(-1), 0, 0, endUs, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
            drain(true)
            check(written > 0) { "the take holds no audio" }
            muxer!!.stop()
            muxing = false
            endUs
          } finally {
            release()
          }
        }
      })
    }, "VoiceFileWriter-finish").apply { isDaemon = true }.start()
  }

  override fun cancel() {
    closed = true
    try { codec?.stop() } catch (_: Throwable) {}
    synchronized(lock) { release() }
  }

  private fun drain(untilEnd: Boolean) {
    val encoder = codec!!
    while (true) {
      val index = encoder.dequeueOutputBuffer(info, if (untilEnd) -1 else 0)
      when {
        index == MediaCodec.INFO_TRY_AGAIN_LATER -> return
        index == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
          track = muxer!!.addTrack(encoder.outputFormat)
          muxer!!.start()
          muxing = true
        }
        index >= 0 -> {
          val config = info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
          if (info.size > 0 && !config) {
            val output = encoder.getOutputBuffer(index)!!.apply { position(info.offset); limit(info.offset + info.size) }
            muxer!!.writeSampleData(track, output, info)
            written++
          }
          encoder.releaseOutputBuffer(index, false)
          if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) return
        }
      }
    }
  }

  private fun release() {
    closed = true
    try { codec?.stop() } catch (_: Throwable) {}
    try { codec?.release() } catch (_: Throwable) {}
    try { if (muxing) muxer?.stop() } catch (_: Throwable) {}
    try { muxer?.release() } catch (_: Throwable) {}
    codec = null
    muxer = null
    muxing = false
  }
}
