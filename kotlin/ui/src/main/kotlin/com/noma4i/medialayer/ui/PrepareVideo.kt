package com.noma4i.medialayer.ui

import com.noma4i.medialayer.normalizeItem
import com.noma4i.medialayer.toItem
import android.content.Context
import android.media.MediaCodecInfo
import android.media.MediaExtractor
import android.media.MediaFormat
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.os.Handler
import android.os.Looper
import androidx.media3.common.Effect
import androidx.media3.common.MediaItem
import androidx.media3.common.MimeTypes
import androidx.media3.common.audio.AudioProcessor
import androidx.media3.common.audio.ChannelMixingAudioProcessor
import androidx.media3.common.audio.ChannelMixingMatrix
import androidx.media3.common.audio.SonicAudioProcessor
import androidx.media3.common.util.UnstableApi
import androidx.media3.effect.FrameDropEffect
import androidx.media3.effect.Presentation
import androidx.media3.transformer.AudioEncoderSettings
import androidx.media3.transformer.Composition
import androidx.media3.transformer.DefaultEncoderFactory
import androidx.media3.transformer.EditedMediaItem
import androidx.media3.transformer.Effects
import androidx.media3.transformer.ExportException
import androidx.media3.transformer.ExportResult
import androidx.media3.transformer.InAppMp4Muxer
import androidx.media3.transformer.ProgressHolder
import androidx.media3.transformer.Transformer
import androidx.media3.transformer.VideoEncoderSettings
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.VideoLimits
import com.noma4i.medialayer.VideoPlan
import com.noma4i.medialayer.VideoSource
import com.noma4i.medialayer.defaultVideoLimits
import com.noma4i.medialayer.planVideo
import java.io.File
import java.io.IOException
import java.util.UUID
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext

/**
 * A video made to fit the limits: H.264 and AAC in mp4, upright, with even sides. A file that already fits comes back untouched.
 * `onProgress` gets 0 to 1 while the export runs.
 */
suspend fun MediaLayer.prepareVideo(
  source: MediaSource,
  limits: VideoLimits = defaultVideoLimits(),
  onProgress: ((Double) -> Unit)? = null
): PreparedMedia {
  val context = checkNotNull(binding) { "the layer was not opened with MediaLayer.open(context)" }.factory.appContext
  return prepareVideoAt(context, preparableFile(normalizeItem(source.toItem()).url), limits, onProgress)
}

internal suspend fun prepareVideoAt(context: Context, file: File, limits: VideoLimits, onProgress: ((Double) -> Unit)?): PreparedMedia {
  val facts = withContext(prepareDispatcher) { videoReader(file) }
  val plan = planVideo(facts.toSource(), limits)
  if (!plan.reencode) return PreparedMedia(file, false)
  val output = File(context.cacheDir, "prepared-${UUID.randomUUID()}.mp4")
  try {
    videoExporter(VideoExport(context, file, output, transformSetup(plan, limits, facts), onProgress))
  } catch (error: Throwable) {
    withContext(NonCancellable + prepareDispatcher) { output.delete() }
    throw error
  }
  onProgress?.invoke(1.0)
  return PreparedMedia(output, true)
}

internal class VideoFacts(
  val width: Int,
  val height: Int,
  val rotation: Int,
  val frameRate: Float,
  val bitrate: Int?,
  val audioChannels: Int?,
  val mp4H264Aac: Boolean
) {
  fun toSource() = VideoSource(width.toUInt(), height.toUInt(), rotation, frameRate, bitrate?.toUInt(), mp4H264Aac)
}

internal class VideoExport(val context: Context, val source: File, val output: File, val setup: TransformSetup, val onProgress: ((Double) -> Unit)?)

internal class TransformSetup(
  val videoEffects: List<Effect>,
  val audioProcessors: List<AudioProcessor>,
  val videoBitrate: Int,
  val audioBitrate: Int,
  val videoMime: String,
  val audioMime: String
)

internal var videoReader: (File) -> VideoFacts = ::readVideoFacts

internal var videoExporter: suspend (VideoExport) -> Unit = ::exportWithTransformer

@androidx.annotation.OptIn(UnstableApi::class)
internal fun transformSetup(plan: VideoPlan, limits: VideoLimits, facts: VideoFacts): TransformSetup {
  val turned = plan.rotationDegrees.toInt() % 180 != 0
  val uprightWidth = if (turned) facts.height else facts.width
  val uprightHeight = if (turned) facts.width else facts.height
  val video = buildList<Effect> {
    if (plan.width.toInt() != uprightWidth || plan.height.toInt() != uprightHeight) {
      add(Presentation.createForWidthAndHeight(plan.width.toInt(), plan.height.toInt(), Presentation.LAYOUT_SCALE_TO_FIT))
    }
    if (facts.frameRate > plan.frameRate.toInt() + 0.5f) add(FrameDropEffect.createDefaultFrameDropEffect(plan.frameRate.toFloat()))
  }
  val audio = buildList<AudioProcessor> {
    if (facts.audioChannels == 1) {
      add(ChannelMixingAudioProcessor().apply { putChannelMixingMatrix(ChannelMixingMatrix.createForConstantGain(1, limits.audioChannels.toInt())) })
    }
    add(SonicAudioProcessor().apply { setOutputSampleRateHz(limits.audioSampleRate.toInt()) })
  }
  return TransformSetup(video, audio, plan.bitrate.toInt(), limits.audioBitrate.toInt(), MimeTypes.VIDEO_H264, MimeTypes.AUDIO_AAC)
}

internal fun readVideoFacts(file: File): VideoFacts {
  val extractor = MediaExtractor()
  try {
    extractor.setDataSource(file.absolutePath)
    val formats = (0 until extractor.trackCount).map(extractor::getTrackFormat)
    val video = formats.firstOrNull { it.getString(MediaFormat.KEY_MIME).orEmpty().startsWith("video/") }
      ?: throw MediaException.Fetch("${file.path} has no video track")
    val audio = formats.firstOrNull { it.getString(MediaFormat.KEY_MIME).orEmpty().startsWith("audio/") }
    val retriever = MediaMetadataRetriever()
    val (durationMs, frames) = try {
      retriever.setDataSource(file.absolutePath)
      (retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_DURATION)?.toLongOrNull() ?: 0L) to
        (retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_FRAME_COUNT)?.toLongOrNull() ?: 0L)
    } finally {
      retriever.release()
    }
    val trackRate = video.takeIf { it.containsKey(MediaFormat.KEY_FRAME_RATE) }?.getNumber(MediaFormat.KEY_FRAME_RATE)?.toFloat()
    val countedRate = if (durationMs > 0 && frames > 0) frames * 1000f / durationMs else 0f
    val trackBitrate = video.takeIf { it.containsKey(MediaFormat.KEY_BIT_RATE) }?.getNumber(MediaFormat.KEY_BIT_RATE)?.toInt()
    val bitrate = trackBitrate ?: if (durationMs > 0) (file.length() * 8000 / durationMs).toInt() else 0
    val fits = file.extension.lowercase() in setOf("mp4", "m4v") &&
      video.getString(MediaFormat.KEY_MIME) == MimeTypes.VIDEO_H264 &&
      (audio == null || audio.getString(MediaFormat.KEY_MIME) == MimeTypes.AUDIO_AAC)
    return VideoFacts(
      video.getInteger(MediaFormat.KEY_WIDTH), video.getInteger(MediaFormat.KEY_HEIGHT),
      if (video.containsKey(MediaFormat.KEY_ROTATION)) video.getInteger(MediaFormat.KEY_ROTATION) else 0,
      trackRate ?: countedRate, bitrate.takeIf { it > 0 }, audio?.getInteger(MediaFormat.KEY_CHANNEL_COUNT), fits
    )
  } finally {
    extractor.release()
  }
}

/** Reads the export progress every `intervalMs` while `block` runs and never after it ends, by completion or by cancellation. */
internal suspend fun <T> whileExporting(
  intervalMs: Long = 100,
  read: () -> Double?,
  report: (Double) -> Unit,
  onTick: () -> Unit = {},
  block: suspend () -> T
): T = coroutineScope {
  val ticker = launch {
    while (true) {
      delay(intervalMs)
      onTick()
      read()?.let(report)
    }
  }
  try {
    block()
  } finally {
    ticker.cancel()
  }
}

@androidx.annotation.OptIn(UnstableApi::class)
private suspend fun exportWithTransformer(request: VideoExport) = withContext(Dispatchers.Main.immediate) {
  val setup = request.setup
  val item = EditedMediaItem.Builder(MediaItem.fromUri(Uri.fromFile(request.source))).setEffects(Effects(setup.audioProcessors, setup.videoEffects)).build()
  val encoders = DefaultEncoderFactory.Builder(request.context)
    .setRequestedVideoEncoderSettings(
      VideoEncoderSettings.Builder().setBitrate(setup.videoBitrate)
        .setEncodingProfileLevel(MediaCodecInfo.CodecProfileLevel.AVCProfileHigh, MediaCodecInfo.CodecProfileLevel.AVCLevel42).build()
    )
    .setRequestedAudioEncoderSettings(AudioEncoderSettings.Builder().setBitrate(setup.audioBitrate).build())
    .build()
  val handler = Handler(Looper.getMainLooper())
  val holder = ProgressHolder()
  var running: Transformer? = null
  whileExporting(
    read = { running?.takeIf { it.getProgress(holder) == Transformer.PROGRESS_STATE_AVAILABLE }?.let { holder.progress / 100.0 } },
    report = { request.onProgress?.invoke(it) }
  ) {
    suspendCancellableCoroutine { continuation ->
      val transformer = Transformer.Builder(request.context)
        .setVideoMimeType(setup.videoMime).setAudioMimeType(setup.audioMime)
        .setEncoderFactory(encoders).setPortraitEncodingEnabled(true)
        .setMuxerFactory(InAppMp4Muxer.Factory().setAttemptStreamableOutputEnabled(true))
        .addListener(object : Transformer.Listener {
          override fun onCompleted(composition: Composition, exportResult: ExportResult) {
            if (continuation.isActive) continuation.resume(Unit)
          }

          override fun onError(composition: Composition, exportResult: ExportResult, exportException: ExportException) {
            if (continuation.isActive) continuation.resumeWithException(IOException("the video cannot be encoded", exportException))
          }
        }).build()
      running = transformer
      continuation.invokeOnCancellation { handler.post { transformer.cancel() } }
      transformer.start(item, request.output.absolutePath)
    }
  }
}
