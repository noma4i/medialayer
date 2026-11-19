package com.noma4i.medialayer.ui

import android.media.MediaExtractor
import android.media.MediaFormat
import com.noma4i.medialayer.AudioCoverage
import com.noma4i.medialayer.AudioInterval
import com.noma4i.medialayer.CoverageConfig
import com.noma4i.medialayer.VideoBounds
import com.noma4i.medialayer.coverageBegin
import com.noma4i.medialayer.coverageFinish
import com.noma4i.medialayer.coveragePush
import java.io.File
import java.nio.ByteBuffer
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ensureActive
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

/** The end of the last video frame, the rule of iOS: the container end from the first PTS, or the last packet PTS when the muxer rounded the duration a few microseconds short of it. Null when a packet or the duration is missing. */
internal fun videoBoundsOf(first: Long, last: Long, duration: Long): VideoBounds? {
  if (first < 0 || last < first || duration <= 0) return null
  return VideoBounds(first, maxOf(Math.addExact(first, duration), last))
}

/** The bounds of the video track of a clip: its first PTS and the end of its last frame (see `videoBoundsOf`). Null when the track, its duration or a timestamp is missing: the clip is then broken, the end is never guessed from a frame step. */
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
    return videoBoundsOf(first, last, duration)
  } catch (_: ArithmeticException) {
    return null
  } finally {
    extractor.release()
  }
}

/** Reads the packet table of the audio of a clip, no PCM is decoded: every packet is one interval of its track frame at its PTS, run through the core rule one by one, the state of the scan is one value. A packet that cannot be read, a negative PTS or a frame that is not AAC-LC is a broken audio, a cancelled read releases the file. */
internal suspend fun scanAudioCoverage(file: File): AudioCoverage = withContext(Dispatchers.IO) {
  val video = try { videoBounds(file) } catch (failure: RuntimeException) { null } ?: return@withContext broken()
  val extractor = MediaExtractor()
  try {
    extractor.setDataSource(file.absolutePath)
    val track = (0 until extractor.trackCount).firstOrNull { extractor.getTrackFormat(it).getString(MediaFormat.KEY_MIME)?.startsWith("audio/") == true }
    if (track == null) return@withContext coverageFinish(coverageBegin(CoverageConfig(true, false, 0u, 0u)), video)
    val format = extractor.getTrackFormat(track)
    val rate = format.getInteger(MediaFormat.KEY_SAMPLE_RATE)
    val frame = if (format.getString(MediaFormat.KEY_MIME) == MediaFormat.MIMETYPE_AUDIO_AAC) aacFrameSamples(format.getByteBuffer("csd-0"), rate) else 0
    var scan = coverageBegin(CoverageConfig(true, true, rate.toUInt(), frame.toUInt()))
    if (frame == 0) return@withContext coverageFinish(scan, video)
    extractor.selectTrack(track)
    val span = frame.toLong() * 1_000_000L / rate
    while (extractor.sampleTrackIndex >= 0) {
      ensureActive()
      val time = extractor.sampleTime
      if (time < 0) return@withContext broken()
      scan = coveragePush(scan, AudioInterval(time, Math.addExact(time, span), frame.toULong()))
      if (!extractor.advance()) break
    }
    coverageFinish(scan, video)
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
