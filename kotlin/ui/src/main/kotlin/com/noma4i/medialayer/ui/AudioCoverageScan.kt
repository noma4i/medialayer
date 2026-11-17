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

/** The frame of one AAC access unit in PCM samples per channel for the AAC-LC the writers of the camera produce: the AudioSpecificConfig is read by media3 `AacUtil` (the object type is its `mp4a.40.2` codec string, the frame is `AAC_LC_AUDIO_SAMPLE_COUNT`) and its rate must be the rate the decoder outputs, so a signalled SBR or PS that doubles the output is not taken for LC. Any other profile or a config that does not parse is 0: the core then refuses the clip as broken. */
@androidx.annotation.OptIn(androidx.media3.common.util.UnstableApi::class)
internal fun aacFrameSamples(config: ByteBuffer?, outputRate: Int): Int {
  if (config == null) return 0
  val bytes = ByteArray(config.remaining()).also { config.duplicate().get(it) }
  val parsed = try { androidx.media3.extractor.AacUtil.parseAudioSpecificConfig(bytes) } catch (_: androidx.media3.common.ParserException) { return 0 }
  return if (parsed.codecs == "mp4a.40.${androidx.media3.extractor.AacUtil.AUDIO_OBJECT_TYPE_AAC_LC}" && parsed.sampleRateHz == outputRate) androidx.media3.extractor.AacUtil.AAC_LC_AUDIO_SAMPLE_COUNT else 0
}

private fun broken(): AudioCoverage = coverageFinish(coverageBegin(CoverageConfig(true, true, 0u, 0u)), null)

/** The bounds of the video track of a clip: its first PTS and the track duration of the container taken from that origin, checked against the real packets (the last PTS lies inside it). Null when the track, its duration or a timestamp is missing or contradicts the packets: the clip is then broken, the end is never guessed from a frame step. */
private fun videoBounds(file: File): VideoBounds? {
  val extractor = MediaExtractor()
  try {
    extractor.setDataSource(file.absolutePath)
    val track = (0 until extractor.trackCount).firstOrNull { extractor.getTrackFormat(it).getString(MediaFormat.KEY_MIME)?.startsWith("video/") == true } ?: return null
    val format = extractor.getTrackFormat(track)
    if (!format.containsKey(MediaFormat.KEY_DURATION)) return null
    val duration = format.getLong(MediaFormat.KEY_DURATION)
    extractor.selectTrack(track)
    var first = -1L
    var last = -1L
    while (extractor.sampleTrackIndex >= 0) {
      val time = extractor.sampleTime
      if (time < 0) return null
      if (first < 0) first = time
      last = maxOf(last, time)
      if (!extractor.advance()) break
    }
    if (first < 0 || duration <= 0) return null
    val end = Math.addExact(first, duration)
    return if (last >= end) null else VideoBounds(first, end)
  } catch (_: ArithmeticException) {
    return null
  } finally {
    extractor.release()
  }
}

/** Decodes the audio of a clip once and runs its PCM through the core rule: the state of the scan is one value, no PCM and no interval is kept. A decode error is a broken audio, a cancelled read releases the codec and the file. */
internal suspend fun scanAudioCoverage(file: File): AudioCoverage = withContext(Dispatchers.IO) {
  val video = try { videoBounds(file) } catch (failure: RuntimeException) { null } ?: return@withContext broken()
  val extractor = MediaExtractor()
  try {
    extractor.setDataSource(file.absolutePath)
    val track = (0 until extractor.trackCount).firstOrNull { extractor.getTrackFormat(it).getString(MediaFormat.KEY_MIME)?.startsWith("audio/") == true }
    if (track == null) return@withContext coverageFinish(coverageBegin(CoverageConfig(true, false, 0u, 0u)), video)
    val format = extractor.getTrackFormat(track)
    extractor.selectTrack(track)
    decode(extractor, format, video)
  } catch (failure: kotlinx.coroutines.CancellationException) {
    throw failure
  } catch (failure: RuntimeException) {
    broken()
  } catch (failure: java.io.IOException) {
    broken()
  } finally {
    extractor.release()
  }
}

private suspend fun decode(extractor: MediaExtractor, format: MediaFormat, video: VideoBounds): AudioCoverage {
  val codec = MediaCodec.createDecoderByType(format.getString(MediaFormat.KEY_MIME)!!)
  val thread = HandlerThread("coverage-scan").apply { start() }
  val done = CompletableDeferred<Unit>()
  var scan: CoverageScan? = null
  var rate = 0
  var bytesPerFrame = 0
  var broken = false
  var frame = 0
  fun begin(output: MediaFormat) {
    val channels = output.getInteger(MediaFormat.KEY_CHANNEL_COUNT)
    val sampleRate = output.getInteger(MediaFormat.KEY_SAMPLE_RATE)
    val encoding = if (output.containsKey(MediaFormat.KEY_PCM_ENCODING)) output.getInteger(MediaFormat.KEY_PCM_ENCODING) else AudioFormat.ENCODING_PCM_16BIT
    val width = when (encoding) { AudioFormat.ENCODING_PCM_16BIT -> 2; AudioFormat.ENCODING_PCM_FLOAT -> 4; else -> 0 }
    val previous = scan
    if (previous != null) { broken = broken || sampleRate != rate || channels * width != bytesPerFrame; return }
    rate = sampleRate
    frame = if (format.getString(MediaFormat.KEY_MIME) == MediaFormat.MIMETYPE_AUDIO_AAC) aacFrameSamples(format.getByteBuffer("csd-0"), sampleRate) else 0
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
          val end = Math.addExact(info.presentationTimeUs, Math.multiplyExact(samples.toLong(), 1_000_000L) / rate)
          if (info.presentationTimeUs < 0) broken = true
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
    val last = scan ?: coverageBegin(CoverageConfig(true, true, format.getInteger(MediaFormat.KEY_SAMPLE_RATE).toUInt(), 0u))
    return if (broken) coverageFinish(last.copy(defect = last.defect ?: CoverageVerdict.BROKEN), video) else coverageFinish(last, video)
  } catch (failure: kotlinx.coroutines.CancellationException) {
    throw failure
  } catch (failure: Exception) {
    return broken()
  } finally {
    try { codec.stop() } catch (_: Exception) {}
    codec.release()
    thread.quitSafely()
  }
}
