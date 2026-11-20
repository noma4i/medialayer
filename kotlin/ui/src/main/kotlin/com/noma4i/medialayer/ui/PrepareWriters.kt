package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.Rect
import android.media.ExifInterface
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
import androidx.media3.container.MdtaMetadataEntry
import androidx.media3.container.Mp4LocationData
import androidx.media3.container.Mp4TimestampData
import androidx.media3.container.XmpData
import androidx.media3.effect.FrameDropEffect
import androidx.media3.effect.Presentation
import androidx.media3.effect.ScaleAndRotateTransformation
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
import com.noma4i.medialayer.ImageFormat
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.PhotoPlan
import com.noma4i.medialayer.PhotoTurn
import com.noma4i.medialayer.PreparePlan
import com.noma4i.medialayer.PrepareProbe
import com.noma4i.medialayer.VideoSource
import com.noma4i.medialayer.photoTurn
import java.io.File
import java.io.IOException
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlin.math.roundToInt
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext


/** The facts of a video the way the prepare reads them: the rotation is a turn of 0 to 359 degrees. */
internal fun readPrepareVideoFacts(file: File): VideoFacts {
  val facts = readVideoFacts(file)
  return VideoFacts(facts.width, facts.height, facts.rotation.mod(360), facts.frameRate, facts.bitrate, facts.audioChannels, facts.mp4H264Aac, facts.mime, facts.profile)
}

/** The metadata keys of the source, with the location of the container when the retriever finds one. */
internal fun readPrepareMetadataKeys(context: Context, file: File): List<String> {
  val retriever = MediaMetadataRetriever()
  val located = try {
    retriever.setDataSource(file.absolutePath)
    retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_LOCATION) != null
  } finally {
    retriever.release()
  }
  return (readMetadataKeys(context, file) + listOfNotNull("location".takeIf { located })).distinct()
}

internal fun probeOfVideo(facts: VideoFacts, metadataKeys: List<String>, sizeBytes: Long): PrepareProbe =
  PrepareProbe.Video(facts.toSource(), metadataKeys, sizeBytes.toULong())

/** The bounds, the EXIF orientation and the format of an image, or null when the file is not an image. */
internal fun probeOfImage(file: File): PrepareProbe.Image? {
  val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }.also { BitmapFactory.decodeFile(file.path, it) }
  if (bounds.outWidth <= 0 || bounds.outHeight <= 0) return null
  val tag = ExifInterface(file.path).getAttributeInt(ExifInterface.TAG_ORIENTATION, ExifInterface.ORIENTATION_NORMAL).takeIf { it in 1..8 } ?: ExifInterface.ORIENTATION_NORMAL
  val format = if (bounds.outMimeType == "image/png") ImageFormat.PNG else ImageFormat.JPEG
  return PrepareProbe.Image(bounds.outWidth.toUInt(), bounds.outHeight.toUInt(), tag.toUInt(), format)
}

@androidx.annotation.OptIn(UnstableApi::class)
internal fun transformSetup(prepare: PreparePlan.Video, facts: VideoFacts, encode: Boolean): TransformSetup {
  val plan = prepare.plan
  if (!encode) return TransformSetup(emptyList(), emptyList(), plan.bitrate.toInt(), prepare.audioBitrate.toInt(), null, null)
  val turned = plan.rotationDegrees.toInt() % 180 != 0
  val uprightWidth = if (turned) facts.height else facts.width
  val uprightHeight = if (turned) facts.width else facts.height
  val video = buildList<Effect> {
    if (plan.width.toInt() != uprightWidth || plan.height.toInt() != uprightHeight) {
      add(Presentation.createForWidthAndHeight(plan.width.toInt(), plan.height.toInt(), Presentation.LAYOUT_SCALE_TO_FIT))
    }
    if (facts.frameRate > plan.frameRate.toInt() + 0.5f) add(FrameDropEffect.createDefaultFrameDropEffect(plan.frameRate.toFloat()))
    if (prepare.mirror) add(ScaleAndRotateTransformation.Builder().setScale(-1f, 1f).build())
  }
  val audio = buildList<AudioProcessor> {
    if (facts.audioChannels == 1) {
      add(ChannelMixingAudioProcessor().apply { putChannelMixingMatrix(ChannelMixingMatrix.createForConstantGain(1, prepare.audioChannels.toInt())) })
    }
    add(SonicAudioProcessor().apply { setOutputSampleRateHz(prepare.audioSampleRate.toInt()) })
  }
  return TransformSetup(video, audio, plan.bitrate.toInt(), prepare.audioBitrate.toInt(), MimeTypes.VIDEO_H264, MimeTypes.AUDIO_AAC)
}

/** Writes the video by the setup: a setup with no mime only repacks the streams without metadata, otherwise Media3 decodes and encodes with the requested settings. */
@androidx.annotation.OptIn(UnstableApi::class)
internal suspend fun exportPrepared(request: VideoExport) = withContext(Dispatchers.Main.immediate) {
  val setup = request.setup
  val item = EditedMediaItem.Builder(MediaItem.fromUri(Uri.fromFile(request.source))).setEffects(Effects(setup.audioProcessors, setup.videoEffects)).build()
  // Requested encoder settings make Media3 decode and re-encode, so a repack must not set them.
  val encoders = DefaultEncoderFactory.Builder(request.context).apply {
    if (setup.videoMime == null) return@apply
    setRequestedVideoEncoderSettings(
      VideoEncoderSettings.Builder().setBitrate(setup.videoBitrate)
        .setEncodingProfileLevel(MediaCodecInfo.CodecProfileLevel.AVCProfileHigh, MediaCodecInfo.CodecProfileLevel.AVCLevel42).build()
    )
    setRequestedAudioEncoderSettings(AudioEncoderSettings.Builder().setBitrate(setup.audioBitrate).build())
  }.build()
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
            if (continuation.isActive) {
              continuation.resumeWithException(IOException("the video cannot be encoded: ${exportException.errorCodeName} ${exportException.cause?.message.orEmpty()}".trim(), exportException))
            }
          }
        }).build()
      running = transformer
      continuation.invokeOnCancellation { handler.post { transformer.cancel() } }
      transformer.start(item, request.output.absolutePath)
    }
  }
}

/** The image of the plan written to [destination]: JPEG on white when the source has alpha, PNG as it is. Returns the mime and the output size, or null when the file does not decode. */
internal suspend fun writePreparedImage(source: File, destination: File, prepare: PreparePlan.Image): Triple<String, Int, Int>? {
  val plan = prepare.plan
  val decoded = BitmapFactory.decodeFile(source.path, BitmapFactory.Options().apply { inSampleSize = plan.sample.toInt() }) ?: return null
  currentCoroutineContext().ensureActive()
  val turned = Bitmap.createBitmap(decoded, 0, 0, decoded.width, decoded.height, turnMatrix(plan.turn), true)
  val drawn = if (plan.needsDraw) redraw(turned, plan) else turned
  val flat = if (prepare.format == ImageFormat.JPEG && drawn.hasAlpha()) onWhite(drawn) else drawn
  try {
    currentCoroutineContext().ensureActive()
    val written = destination.outputStream().use {
      if (prepare.format == ImageFormat.PNG) flat.compress(Bitmap.CompressFormat.PNG, 100, it)
      else flat.compress(Bitmap.CompressFormat.JPEG, (prepare.quality * 100).roundToInt(), it)
    }
    if (!written) throw IOException("the image does not encode")
    return Triple(if (prepare.format == ImageFormat.PNG) "image/png" else "image/jpeg", flat.width, flat.height)
  } finally {
    if (flat !== drawn) flat.recycle()
    if (drawn !== turned) drawn.recycle()
    if (turned !== decoded) turned.recycle()
    decoded.recycle()
  }
}

private fun onWhite(source: Bitmap): Bitmap {
  val result = Bitmap.createBitmap(source.width, source.height, Bitmap.Config.ARGB_8888)
  Canvas(result).apply {
    drawColor(Color.WHITE)
    drawBitmap(source, 0f, 0f, null)
  }
  return result
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

internal class VideoExport(val context: Context, val source: File, val output: File, val setup: TransformSetup, val onProgress: ((Double) -> Unit)?)

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

internal fun metadataKeyOf(entry: Metadata.Entry): String? = when (entry) {
  is Mp4LocationData -> "location"
  is Mp4TimestampData -> "creation_time".takeIf { entry.creationTimestampSeconds != 0L }
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
        (0 until (metadata?.length() ?: 0)).mapNotNull { metadataKeyOf(metadata!![it]) }
      }
    }
  }

/** The muxer keeps no metadata entry of the source: no location, no tag, and a zero (MP4 epoch) creation time instead of a real one. */
internal val dropSourceMetadata = InAppMp4Muxer.MetadataProvider { entries ->
  entries.clear()
  entries.add(Mp4TimestampData(0L, 0L))
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

/** The crop of the plan, moved to the sampled size of [upright], drawn into the output size and flipped when the plan asks for the mirror. */
internal fun redraw(upright: Bitmap, plan: PhotoPlan): Bitmap {
  val width = plan.output.width.toInt()
  val height = plan.output.height.toInt()
  val result = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
  val canvas = Canvas(result)
  if (plan.mirror) canvas.scale(-1f, 1f, width / 2f, 0f)
  val across = upright.width.toDouble() / plan.upright.width.toInt()
  val down = upright.height.toDouble() / plan.upright.height.toInt()
  val left = (plan.crop.x.toInt() * across).roundToInt().coerceIn(0, upright.width - 1)
  val top = (plan.crop.y.toInt() * down).roundToInt().coerceIn(0, upright.height - 1)
  val area = Rect(
    left,
    top,
    ((plan.crop.x + plan.crop.width).toInt() * across).roundToInt().coerceIn(left + 1, upright.width),
    ((plan.crop.y + plan.crop.height).toInt() * down).roundToInt().coerceIn(top + 1, upright.height)
  )
  canvas.drawBitmap(upright, area, Rect(0, 0, width, height), Paint(Paint.FILTER_BITMAP_FLAG))
  return result
}

internal fun turnMatrix(exifOrientation: Int): Matrix = turnMatrix(photoTurn(exifOrientation.toUInt()))

internal fun turnMatrix(turn: PhotoTurn): Matrix =
  Matrix().apply {
    postRotate(turn.rotationDegrees.toFloat())
    if (turn.flipHorizontal) postScale(-1f, 1f)
  }
