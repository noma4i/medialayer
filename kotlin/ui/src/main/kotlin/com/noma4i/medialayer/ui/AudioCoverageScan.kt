package com.noma4i.medialayer.ui

import android.media.AudioFormat
import android.media.MediaCodec
import android.media.MediaExtractor
import android.media.MediaFormat
import android.os.Handler
import android.os.HandlerThread
import com.noma4i.medialayer.AudioCoverage
import com.noma4i.medialayer.AudioInterval
import com.noma4i.medialayer.CoverageConfig
import com.noma4i.medialayer.CoverageScan
import com.noma4i.medialayer.CoverageVerdict
import com.noma4i.medialayer.VideoBounds
import com.noma4i.medialayer.coverageBegin
import com.noma4i.medialayer.coverageFinish
import com.noma4i.medialayer.coveragePush
import java.io.File
import java.nio.ByteBuffer
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** The frame of one AAC access unit in PCM samples per channel, read from the AudioSpecificConfig (ISO/IEC 14496-3, 1.6.2.1 and 4.4.1 GASpecificConfig). 0 when the profile is not one this reads: the core then refuses the clip as broken. */
internal fun aacFrameSamples(config: ByteBuffer?): Int {
  if (config == null) return 0
  val bytes = ByteArray(config.remaining()).also { config.duplicate().get(it) }
  var at = 0
  fun bits(count: Int): Int {
    var value = 0
    repeat(count) {
      if (at >= bytes.size * 8) return -1
      value = value shl 1 or (bytes[at / 8].toInt() shr (7 - at % 8) and 1)
      at++
    }
    return value
  }
  fun objectType(): Int = bits(5).let { if (it == 31) bits(6).let { more -> if (more < 0) -1 else 32 + more } else it }
  fun rate() { if (bits(4) == 15) bits(24) }
  var type = objectType()
  rate()
  bits(4)
  var factor = 1
  if (type == 5 || type == 29) {
    rate()
    type = objectType()
    factor = 2
  }
  val flag = when (type) {
    1, 2, 4, 17, 19 -> bits(1)
    39 -> bits(1)
    else -> -1
  }
  if (flag < 0) return 0
  val frame = if (type == 39) (if (flag == 0) 512 else 480) else (if (flag == 0) 1024 else 960)
  return frame * factor
}

/** The first and the last end of the video samples of a clip: the end of the last one is its PTS plus the real step to it from the one before. */
private fun videoBounds(file: File): VideoBounds? {
  val extractor = MediaExtractor()
  try {
    extractor.setDataSource(file.absolutePath)
    val track = (0 until extractor.trackCount).firstOrNull { extractor.getTrackFormat(it).getString(MediaFormat.KEY_MIME)?.startsWith("video/") == true } ?: return null
    extractor.selectTrack(track)
    var first = -1L
    var last = -1L
    var step = 0L
    while (extractor.sampleTrackIndex >= 0) {
      val time = extractor.sampleTime
      if (first < 0) first = time else if (time > last) step = time - last
      if (time > last) last = time
      if (!extractor.advance()) break
    }
    return if (first < 0) null else VideoBounds(first, last + step)
  } finally {
    extractor.release()
  }
}

/** Decodes the audio of a clip once and runs its PCM through the core rule: the state of the scan is one value, no PCM and no interval is kept. A decode error is a broken audio, a cancelled read releases the codec and the file. */
internal suspend fun scanAudioCoverage(file: File): AudioCoverage = withContext(Dispatchers.IO) {
  val video = videoBounds(file)
  val extractor = MediaExtractor()
  try {
    extractor.setDataSource(file.absolutePath)
    val track = (0 until extractor.trackCount).firstOrNull { extractor.getTrackFormat(it).getString(MediaFormat.KEY_MIME)?.startsWith("audio/") == true }
    if (track == null) return@withContext coverageFinish(coverageBegin(CoverageConfig(true, false, 0u, 0u)), video)
    val format = extractor.getTrackFormat(track)
    extractor.selectTrack(track)
    val frame = if (format.getString(MediaFormat.KEY_MIME) == MediaFormat.MIMETYPE_AUDIO_AAC) aacFrameSamples(format.getByteBuffer("csd-0")) else 0
    decode(extractor, format, frame, video)
  } catch (failure: java.io.IOException) {
    coverageFinish(coverageBegin(CoverageConfig(true, true, 0u, 0u)), video)
  } finally {
    extractor.release()
  }
}

private suspend fun decode(extractor: MediaExtractor, format: MediaFormat, frame: Int, video: VideoBounds?): AudioCoverage {
  val codec = MediaCodec.createDecoderByType(format.getString(MediaFormat.KEY_MIME)!!)
  val thread = HandlerThread("coverage-scan").apply { start() }
  val done = CompletableDeferred<Unit>()
  var scan: CoverageScan? = null
  var rate = 0
  var bytesPerFrame = 0
  var broken = false
  fun begin(output: MediaFormat) {
    val channels = output.getInteger(MediaFormat.KEY_CHANNEL_COUNT)
    val sampleRate = output.getInteger(MediaFormat.KEY_SAMPLE_RATE)
    val encoding = if (output.containsKey(MediaFormat.KEY_PCM_ENCODING)) output.getInteger(MediaFormat.KEY_PCM_ENCODING) else AudioFormat.ENCODING_PCM_16BIT
    val width = when (encoding) { AudioFormat.ENCODING_PCM_16BIT -> 2; AudioFormat.ENCODING_PCM_FLOAT -> 4; else -> 0 }
    val previous = scan
    if (previous != null) { broken = broken || sampleRate != rate || channels * width != bytesPerFrame; return }
    rate = sampleRate
    bytesPerFrame = channels * width
    broken = bytesPerFrame == 0
    scan = coverageBegin(CoverageConfig(true, true, sampleRate.toUInt(), frame.toUInt()))
  }
  codec.setCallback(object : MediaCodec.Callback() {
    override fun onInputBufferAvailable(codec: MediaCodec, index: Int) {
      if (done.isCompleted) return
      try {
        val buffer = codec.getInputBuffer(index) ?: return
        val size = extractor.readSampleData(buffer, 0)
        if (size < 0) codec.queueInputBuffer(index, 0, 0, 0, MediaCodec.BUFFER_FLAG_END_OF_STREAM)
        else { codec.queueInputBuffer(index, 0, size, extractor.sampleTime, 0); extractor.advance() }
      } catch (failure: Exception) { done.completeExceptionally(failure) }
    }

    override fun onOutputBufferAvailable(codec: MediaCodec, index: Int, info: MediaCodec.BufferInfo) {
      try {
        if (!done.isCompleted && info.size > 0 && info.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG == 0) {
          if (scan == null) begin(codec.outputFormat)
          val samples = info.size / bytesPerFrame
          val end = info.presentationTimeUs + samples * 1_000_000L / rate
          if (!broken) scan = coveragePush(scan!!, AudioInterval(info.presentationTimeUs, end, samples.toULong()))
        }
        codec.releaseOutputBuffer(index, false)
        if (info.flags and MediaCodec.BUFFER_FLAG_END_OF_STREAM != 0) done.complete(Unit)
      } catch (failure: Exception) { done.completeExceptionally(failure) }
    }

    override fun onOutputFormatChanged(codec: MediaCodec, output: MediaFormat) {
      try { begin(output) } catch (failure: Exception) { done.completeExceptionally(failure) }
    }

    override fun onError(codec: MediaCodec, error: MediaCodec.CodecException) { done.completeExceptionally(error) }
  }, Handler(thread.looper))
  try {
    codec.configure(format, null, null, 0)
    codec.start()
    done.await()
    val last = scan ?: coverageBegin(CoverageConfig(true, true, format.getInteger(MediaFormat.KEY_SAMPLE_RATE).toUInt(), frame.toUInt()))
    return if (broken) coverageFinish(last.copy(defect = last.defect ?: CoverageVerdict.BROKEN), video) else coverageFinish(last, video)
  } catch (failure: kotlinx.coroutines.CancellationException) {
    throw failure
  } catch (failure: Exception) {
    return coverageFinish(coverageBegin(CoverageConfig(true, true, 0u, 0u)), video)
  } finally {
    try { codec.stop() } catch (_: Exception) {}
    codec.release()
    thread.quitSafely()
  }
}
