package com.noma4i.medialayer.ui

import android.media.AudioFormat
import android.media.MediaCodec
import android.media.MediaFormat
import android.os.Handler
import android.os.HandlerThread
import android.view.Surface
import androidx.media3.common.util.MediaFormatUtil
import androidx.media3.common.util.UnstableApi
import androidx.media3.muxer.BufferInfo
import androidx.media3.muxer.Mp4Muxer
import com.noma4i.medialayer.FailureSource
import com.noma4i.medialayer.NativeFailure
import java.io.File
import java.io.FileOutputStream
import java.nio.ByteBuffer
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

internal enum class VideoTimebase { BOOTTIME, UNKNOWN }

internal class CameraTakeSettings(
  val video: MediaFormat,
  val audio: MediaFormat?,
  val videoTimebase: VideoTimebase,
  val audioGeneration: Long,
  val heldLimitBytes: Long,
  val maxDurationUs: Long
)

internal interface CameraTakeListener {
  fun onBegan(generation: Long, inputSurface: Surface)

  fun onVideoEncoded(generation: Long, ptsUs: Long)

  fun onStarted(generation: Long, baseUs: Long)

  fun onFinished(generation: Long, file: File, durationUs: Long)

  fun onFailed(generation: Long, failure: NativeFailure)

  fun onCancelled(generation: Long)
}

@OptIn(UnstableApi::class)
internal class CameraTake(
  val generation: Long,
  private val file: File,
  private val settings: CameraTakeSettings,
  private val admit: (Long) -> AutoCloseable,
  private val listener: CameraTakeListener
) : CameraTaking {
  private class Sample(val track: Int, val ptsUs: Long, val flags: Int, val bytes: ByteArray)
  private class Pcm(val bytes: ByteArray, var offset: Int, var ptsUs: Long, val bytesPerFrame: Int)

  private val thread = HandlerThread("CameraTake-$generation").apply { start() }
  private val handler = Handler(thread.looper)
  private val ended = AtomicBoolean(false)
  private val held = AtomicLong()
  private val failing = AtomicBoolean()
  private var begun = false
  private val hasAudio = settings.audio != null
  private var reserve: AutoCloseable? = null
  private var videoCodec: MediaCodec? = null
  private var audioCodec: MediaCodec? = null
  private var surface: Surface? = null
  private var videoFormat: androidx.media3.common.Format? = null
  private var audioFormat: androidx.media3.common.Format? = null
  private var muxer: Mp4Muxer? = null
  private var output: FileOutputStream? = null
  private var tracks = IntArray(2)
  private var baseUs = 0L
  private var written = IntArray(2)
  private var heldVideo = ArrayList<Sample>()
  private var heldAudio = ArrayList<Sample>()
  private var lastPts = LongArray(2) { -1L }
  private var tail = ArrayList<Sample>()
  private var needKeyframe = true
  private val pending = ArrayDeque<Pcm>()
  private val freeInputs = ArrayDeque<Int>()
  private var originUs: Long? = null
  private var live = false
  private var confirmed = false
  private var finishing = false
  private var audioEos = false
  private var videoEos = false
  private var audioDone = !hasAudio
  private var videoDone = false
  private var started = false

  override fun begin(): Boolean = handler.post {
    if (begun || ended.get()) return@post
    begun = true
    try {
      if (settings.videoTimebase == VideoTimebase.UNKNOWN) return@post fail(0, "video timebase is unknown, not taken as REALTIME")
      reserve = admit(settings.heldLimitBytes)
      codec(settings.video, VIDEO).also {
        surface = it.createInputSurface()
        it.start()
      }
      settings.audio?.let { format -> codec(format, AUDIO).start() }
      listener.onBegan(generation, surface!!)
    } catch (error: Throwable) {
      fail(0, error.message ?: error.javaClass.name)
    }
  }

  override fun audio(chunk: PcmChunk) {
    if (!hasAudio || chunk.generation != settings.audioGeneration || ended.get() || failing.get()) return
    val rate = settings.audio!!.getInteger(MediaFormat.KEY_SAMPLE_RATE)
    val channels = settings.audio.getInteger(MediaFormat.KEY_CHANNEL_COUNT)
    val size = chunk.data.remaining()
    val bytesPerFrame = Integer.bitCount(chunk.spec.channelMask) * 2
    val mismatch = chunk.spec.encoding != AudioFormat.ENCODING_PCM_16BIT || chunk.spec.sampleRate != rate || bytesPerFrame != channels * 2
    if (mismatch || chunk.frames <= 0 || size != chunk.frames * bytesPerFrame) return failLater("PCM does not match the audio encoder")
    if (!acquire(size.toLong())) return failLater("PCM exceeds the memory reserve")
    val copy = ByteArray(size).also { chunk.data.duplicate().get(it) }
    post {
      if (ended.get() || audioEos) { held.addAndGet(-size.toLong()); return@post }
      live = true
      pending.addLast(Pcm(copy, 0, chunk.ptsUs, bytesPerFrame))
      drain()
      tryStart()
    }
  }

  private fun acquire(bytes: Long): Boolean {
    if (held.addAndGet(bytes) <= settings.heldLimitBytes) return true
    held.addAndGet(-bytes)
    return false
  }

  override fun audioTimebase(generation: Long, originNanos: Long) {
    if (generation != settings.audioGeneration) return
    post { if (ended.get()) return@post; originUs = originNanos / 1000; drain() }
  }

  private fun failLater(message: String) { if (failing.compareAndSet(false, true)) post { fail(0, message) } }

  override fun confirm() { post { confirmed = true; tryStart() } }

  override fun finish() { post { if (ended.get()) return@post; if (!started) return@post fail(0, "the take ended before it started"); beginFinish() } }

  /** Every callback and Runnable of the take runs here: a codec that throws ends the take through the one failure path. */
  private fun post(block: () -> Unit) { handler.post { safely(block) } }

  private fun safely(block: () -> Unit) {
    try { block() } catch (error: Throwable) { fail(0, error.message ?: error.javaClass.name) }
  }

  private fun beginFinish() {
    if (finishing) return
    finishing = true
    videoCodec?.signalEndOfInputStream()
    drain()
  }

  /** After the end of the take its thread is gone: a cancel is a no-op by the state of the take. */
  override fun cancel() { if (!ended.get()) handler.post { end(null, null) } }

  private fun codec(format: MediaFormat, kind: Int): MediaCodec {
    val made = MediaCodec.createEncoderByType(format.getString(MediaFormat.KEY_MIME)!!)
    if (kind == VIDEO) videoCodec = made else audioCodec = made
    made.setCallback(object : MediaCodec.Callback() {
      override fun onInputBufferAvailable(codec: MediaCodec, index: Int) {
        if (ended.get()) return
        safely {
          freeInputs.addLast(index)
          drain()
        }
      }

      override fun onOutputBufferAvailable(codec: MediaCodec, index: Int, info: MediaCodec.BufferInfo) = output(codec, kind, index, info)

      override fun onError(codec: MediaCodec, error: MediaCodec.CodecException) = if (ended.get()) Unit else fail(error.errorCode.toLong(), error.diagnosticInfo)

      override fun onOutputFormatChanged(codec: MediaCodec, format: MediaFormat) {
        if (ended.get()) return
        safely {
          val converted = MediaFormatUtil.createFormatFromMediaFormat(format)
          if (kind == VIDEO) videoFormat = converted else audioFormat = converted
          tryStart()
        }
      }
    }, handler)
    made.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
    return made
  }

  private fun drain() {
    val codec = audioCodec ?: return
    val origin = originUs ?: return
    while (!ended.get() && originUs != null && freeInputs.isNotEmpty() && (pending.isNotEmpty() || (finishing && !audioEos))) {
      val head = pending.firstOrNull()
      if (head == null && !audioReachesVideoEnd()) return
      val index = freeInputs.removeFirst()
      if (head == null) {
        codec.queueInputBuffer(index, 0, 0, lastAudioInputUs, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
        audioEos = true
        return
      }
      val buffer = codec.getInputBuffer(index)!!
      val size = minOf(buffer.remaining(), head.bytes.size - head.offset).let { it - it % head.bytesPerFrame }
      if (size <= 0) return fail(0, "audio input buffer holds no whole frame")
      buffer.put(head.bytes, head.offset, size)
      lastAudioInputUs = origin + head.ptsUs
      codec.queueInputBuffer(index, 0, size, lastAudioInputUs, 0)
      audioEndUs = lastAudioInputUs + size / head.bytesPerFrame * 1_000_000L / settings.audio!!.getInteger(MediaFormat.KEY_SAMPLE_RATE)
      head.offset += size
      head.ptsUs += size / head.bytesPerFrame * 1_000_000L / settings.audio!!.getInteger(MediaFormat.KEY_SAMPLE_RATE)
      if (head.offset >= head.bytes.size) {
        pending.removeFirst()
        held.addAndGet(-head.bytes.size.toLong())
      }
    }
  }

  private var lastAudioInputUs = 0L
  private var audioEndUs = 0L
  private var videoEndUs = -1L

  private var sourceEnded = false

  /** Audio EOS waits for the PCM to cover the end of the last video sample, or for the end of the source. */
  private fun audioReachesVideoEnd(): Boolean = sourceEnded || (videoDone && videoEndUs >= 0 && audioEndUs - baseUs >= videoEndUs)

  override fun audioEnded() { post { sourceEnded = true; drain() } }


  private fun output(codec: MediaCodec, kind: Int, index: Int, info: MediaCodec.BufferInfo) {
    if (ended.get()) return
    try {
      val eos = info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0
      val config = info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG != 0
      if (info.size > 0 && !config && !ended.get()) {
        val buffer = codec.getOutputBuffer(index)!!.apply { position(info.offset); limit(info.offset + info.size) }
        sample(kind, info.presentationTimeUs, info.flags, buffer)
      }
      codec.releaseOutputBuffer(index, false)
      if (kind == VIDEO && info.size > 0 && !config) listener.onVideoEncoded(generation, info.presentationTimeUs)
      if (eos) {
        if (kind == VIDEO) videoDone = true else audioDone = true
        if (kind == VIDEO && written[VIDEO] == 0) return fail(0, "the take ended without video")
        if (kind == VIDEO) { dropTail(); drain() }
        if (audioDone && videoDone) complete()
      }
    } catch (error: Throwable) {
      fail(0, error.message ?: error.javaClass.name)
    }
  }

  private fun sample(kind: Int, ptsUs: Long, flags: Int, buffer: ByteBuffer) {
    if (started) return write(kind, ptsUs - baseUs, flags, buffer)
    val key = flags and MediaCodec.BUFFER_FLAG_KEY_FRAME != 0
    if (kind == VIDEO && needKeyframe && !key) return
    val size = buffer.remaining().toLong()
    if (!acquire(size)) {
      if (kind == AUDIO) return fail(0, "held audio exceeds the memory reserve")
      heldVideo.forEach { held.addAndGet(-it.bytes.size.toLong()) }
      heldVideo.clear()
      needKeyframe = true
      if (!key || !acquire(size)) return
    }
    val bytes = ByteArray(size.toInt()).also { buffer.get(it) }
    if (kind == VIDEO) { needKeyframe = false; heldVideo.add(Sample(VIDEO, ptsUs, flags, bytes)) } else heldAudio.add(Sample(AUDIO, ptsUs, flags, bytes))
    tryStart()
  }

  private fun tryStart() {
    if (started || ended.get() || heldVideo.isEmpty() || videoFormat == null || !confirmed || (hasAudio && !live)) return
    val base = heldVideo.first().ptsUs
    val audible = heldAudio.filter { it.ptsUs >= base }
    if (hasAudio && (audioFormat == null || audible.isEmpty())) return
    try {
      baseUs = base
      val stream = FileOutputStream(file).also { output = it }
      val opened = Mp4Muxer.Builder(stream).build().also { muxer = it }
      tracks[VIDEO] = opened.addTrack(videoFormat!!)
      if (hasAudio) tracks[AUDIO] = opened.addTrack(audioFormat!!)
      started = true
      heldVideo.forEach { write(VIDEO, it.ptsUs - baseUs, it.flags, ByteBuffer.wrap(it.bytes)) }
      audible.forEach { write(AUDIO, it.ptsUs - baseUs, it.flags, ByteBuffer.wrap(it.bytes)) }
      clearHeld()
      listener.onStarted(generation, baseUs)
    } catch (error: Throwable) {
      fail(0, error.message ?: error.javaClass.name)
    }
  }

  private fun clearHeld() {
    (heldVideo + heldAudio).forEach { held.addAndGet(-it.bytes.size.toLong()) }
    heldVideo = ArrayList()
    heldAudio = ArrayList()
  }

  private fun write(kind: Int, ptsUs: Long, flags: Int, buffer: ByteBuffer) {
    if (ptsUs < 0) return
    if (ptsUs > settings.maxDurationUs) return beginFinish()
    if (kind == AUDIO) return writeAudio(ptsUs, flags, buffer)
    mux(VIDEO, ptsUs, flags, buffer)
    val interval = if (lastPts[VIDEO] >= 0) ptsUs - lastPts[VIDEO] else 0L
    lastPts[VIDEO] = ptsUs
    videoEndUs = ptsUs + interval
    flushTail()
  }

  private fun mux(kind: Int, ptsUs: Long, flags: Int, buffer: ByteBuffer) {
    muxer!!.writeSampleData(tracks[kind], buffer, BufferInfo(ptsUs, buffer.remaining(), flags))
    written[kind]++
  }

  /** INV-CAPTURE-51: a take ends at the end of its last written video frame; sound that starts at or after it is not in the file. */
  private fun writeAudio(ptsUs: Long, flags: Int, buffer: ByteBuffer) {
    if (ptsUs < videoEndUs) return mux(AUDIO, ptsUs, flags, buffer)
    if (videoDone) return
    val bytes = ByteArray(buffer.remaining()).also { buffer.get(it) }
    held.addAndGet(bytes.size.toLong())
    tail.add(Sample(AUDIO, ptsUs, flags, bytes))
  }

  private fun flushTail() {
    val ready = tail.filter { it.ptsUs < videoEndUs }
    if (ready.isEmpty()) return
    tail = ArrayList(tail.filter { it.ptsUs >= videoEndUs })
    ready.forEach {
      held.addAndGet(-it.bytes.size.toLong())
      mux(AUDIO, it.ptsUs, it.flags, ByteBuffer.wrap(it.bytes))
    }
  }

  private fun dropTail() {
    tail.forEach { held.addAndGet(-it.bytes.size.toLong()) }
    tail = ArrayList()
  }

  private fun complete() {
    if (!started || written[VIDEO] == 0 || (hasAudio && written[AUDIO] == 0)) return fail(0, "the take ended without both streams")
    end(file, videoEndUs)
  }

  private fun fail(code: Long, message: String) = end(null, null, NativeFailure(FailureSource.RECORDER, "android.media.MediaCodec", code, message))

  private fun end(done: File?, durationUs: Long?, failure: NativeFailure? = null) {
    if (!ended.compareAndSet(false, true)) return
    var outcome = failure
    fun guarded(step: () -> Unit) {
      try { step() } catch (error: Throwable) { if (outcome == null && done != null) outcome = NativeFailure(FailureSource.RECORDER, "CameraTake.end", 0, error.message ?: error.javaClass.name) }
    }
    guarded { muxer?.close() }
    guarded { output?.close() }
    listOf(videoCodec, audioCodec).forEach { codec ->
      guarded { codec?.stop() }
      guarded { codec?.release() }
    }
    guarded { surface?.release() }
    guarded { reserve?.close() }
    pending.clear()
    freeInputs.clear()
    clearHeld()
    dropTail()
    val success = done != null && outcome == null
    if (!success) file.delete()
    try {
      when {
        success -> listener.onFinished(generation, file, durationUs!!)
        outcome != null -> listener.onFailed(generation, outcome!!)
        else -> listener.onCancelled(generation)
      }
    } finally {
      thread.quitSafely()
    }
  }

  private companion object {
    const val VIDEO = 0
    const val AUDIO = 1
  }
}
