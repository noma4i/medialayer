package com.noma4i.medialayer.ui

import com.noma4i.medialayer.normalizeItem
import com.noma4i.medialayer.toItem
import android.content.Context
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaExtractor
import android.media.MediaFormat
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.os.Handler
import android.os.Looper
import androidx.media3.common.Effect
import androidx.media3.common.MediaItem
import androidx.media3.common.Metadata
import androidx.media3.common.MimeTypes
import androidx.media3.common.audio.AudioProcessor
import androidx.media3.common.audio.ChannelMixingAudioProcessor
import androidx.media3.common.audio.ChannelMixingMatrix
import androidx.media3.common.audio.SonicAudioProcessor
import androidx.media3.common.util.UnstableApi
import androidx.media3.effect.FrameDropEffect
import androidx.media3.container.MdtaMetadataEntry
import androidx.media3.container.Mp4LocationData
import androidx.media3.container.Mp4TimestampData
import androidx.media3.container.XmpData
import androidx.media3.effect.Presentation
import androidx.media3.extractor.metadata.id3.Id3Frame
import androidx.media3.inspector.MetadataRetriever
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
import com.noma4i.medialayer.videoMetadataToStrip
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
 * A video made to fit the limits: H.264 and AAC in mp4, upright, with even sides. A file that already fits comes back untouched, or copied without encoding when it carries location or personal tags. The result never carries them.
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
  if (!plan.reencode && videoMetadataToStrip(withContext(prepareDispatcher) { metadataReader(context, file) }).isEmpty()) return PreparedMedia(file, false)
  val output = File(context.cacheDir, "prepared-${UUID.randomUUID()}.mp4")
  try {
    videoExporter(VideoExport(context, file, output, transformSetup(plan, limits, facts), facts, onProgress))
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
  val mp4H264Aac: Boolean,
  val mime: String = MimeTypes.VIDEO_H264,
  val profile: Int? = null
) {
  fun toSource() = VideoSource(width.toUInt(), height.toUInt(), rotation, frameRate, bitrate?.toUInt(), mp4H264Aac)
}

internal class VideoExport(val context: Context, val source: File, val output: File, val setup: TransformSetup, val facts: VideoFacts, val onProgress: ((Double) -> Unit)?)

internal class TransformSetup(
  val videoEffects: List<Effect>,
  val audioProcessors: List<AudioProcessor>,
  val videoBitrate: Int,
  val audioBitrate: Int,
  val videoMime: String?,
  val audioMime: String?
)

internal class DecoderAbility(val accepts: (Int, Int) -> Boolean, val profiles: Set<Int>, val maxWidth: Int, val maxHeight: Int)

internal fun readDecoders(mime: String): List<DecoderAbility> =
  MediaCodecList(MediaCodecList.REGULAR_CODECS).codecInfos
    .filter { !it.isEncoder && !it.name.endsWith(".secure") && it.supportedTypes.any { type -> type.equals(mime, ignoreCase = true) } }
    .mapNotNull { info ->
      val capabilities = info.getCapabilitiesForType(mime)
      val video = capabilities.videoCapabilities ?: return@mapNotNull null
      DecoderAbility(video::isSizeSupported, capabilities.profileLevels.map { it.profile }.toSet(), video.supportedWidths.upper, video.supportedHeights.upper)
    }

/** Why no decoder of the device can read the source, or null when one can. */
internal fun decoderRefusal(facts: VideoFacts, decoders: List<DecoderAbility>): String? {
  val sized = decoders.filter { it.accepts(facts.width, facts.height) }
  if (sized.any { facts.profile == null || facts.profile in it.profiles }) return null
  val size = "${facts.width}x${facts.height}"
  if (sized.isNotEmpty()) return "the device has no ${facts.mime} decoder for profile ${facts.profile} at $size"
  val widest = decoders.maxByOrNull { it.maxWidth.toLong() * it.maxHeight } ?: return "the device has no ${facts.mime} decoder"
  return "the device decodes ${facts.mime} up to ${widest.maxWidth}x${widest.maxHeight}, the video is $size"
}

internal var videoReader: (File) -> VideoFacts = ::readVideoFacts

internal var metadataReader: (Context, File) -> List<String> = ::readMetadataKeys

internal var videoExporter: suspend (VideoExport) -> Unit = ::exportWithTransformer

@androidx.annotation.OptIn(UnstableApi::class)
internal fun transformSetup(plan: VideoPlan, limits: VideoLimits, facts: VideoFacts): TransformSetup {
  if (!plan.reencode) return TransformSetup(emptyList(), emptyList(), plan.bitrate.toInt(), limits.audioBitrate.toInt(), null, null)
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

internal fun metadataKeyOf(entry: Metadata.Entry): String = when (entry) {
  is Mp4LocationData -> "location"
  is Mp4TimestampData -> "creation_time"
  is MdtaMetadataEntry -> entry.key
  is Id3Frame -> entry.id
  is XmpData -> "xmp"
  else -> entry.javaClass.simpleName
}

@androidx.annotation.OptIn(UnstableApi::class)
internal fun readMetadataKeys(context: Context, file: File): List<String> =
  MetadataRetriever.Builder(context, MediaItem.fromUri(Uri.fromFile(file))).build().use { retriever ->
    val groups = retriever.retrieveTrackGroups().get()
    (0 until groups.length).flatMap { index ->
      val group = groups[index]
      (0 until group.length).flatMap { track ->
        val metadata = group.getFormat(track).metadata
        (0 until (metadata?.length() ?: 0)).map { metadataKeyOf(metadata!![it]) }
      }
    }
  }

/** The muxer keeps no metadata entry of the source: no location, no creation time, no tag. */
internal val dropSourceMetadata = InAppMp4Muxer.MetadataProvider { entries -> entries.clear() }

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
      trackRate ?: countedRate, bitrate.takeIf { it > 0 }, audio?.getInteger(MediaFormat.KEY_CHANNEL_COUNT), fits,
      video.getString(MediaFormat.KEY_MIME).orEmpty(),
      video.takeIf { it.containsKey(MediaFormat.KEY_PROFILE) }?.getInteger(MediaFormat.KEY_PROFILE)
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
private suspend fun exportWithTransformer(request: VideoExport) {
  withContext(prepareDispatcher) { decoderRefusal(request.facts, readDecoders(request.facts.mime)) }?.let { throw MediaException.Fetch(it) }
  exportOnMain(request)
}

@androidx.annotation.OptIn(UnstableApi::class)
private suspend fun exportOnMain(request: VideoExport) = withContext(Dispatchers.Main.immediate) {
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
        .apply { setup.videoMime?.let(::setVideoMimeType) }.apply { setup.audioMime?.let(::setAudioMimeType) }
        .setEncoderFactory(encoders).setPortraitEncodingEnabled(true)
        .setMuxerFactory(InAppMp4Muxer.Factory(dropSourceMetadata).setAttemptStreamableOutputEnabled(true))
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
