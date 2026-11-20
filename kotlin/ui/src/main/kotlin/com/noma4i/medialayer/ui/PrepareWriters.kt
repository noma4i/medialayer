package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Canvas
import android.graphics.Color
import android.media.ExifInterface
import android.media.MediaCodecInfo
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
import androidx.media3.effect.ScaleAndRotateTransformation
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
import com.noma4i.medialayer.PrepareProbe
import com.noma4i.medialayer.PreparePlan
import java.io.File
import java.io.IOException
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlin.math.roundToInt
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
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
