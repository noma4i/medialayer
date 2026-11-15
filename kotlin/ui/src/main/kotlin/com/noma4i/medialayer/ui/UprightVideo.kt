package com.noma4i.medialayer.ui

import android.content.Context
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.os.Handler
import android.os.Looper
import androidx.media3.common.MediaItem
import androidx.media3.common.util.UnstableApi
import androidx.media3.transformer.Composition
import androidx.media3.transformer.DefaultEncoderFactory
import androidx.media3.transformer.ExportException
import androidx.media3.transformer.ExportResult
import androidx.media3.transformer.Transformer
import androidx.media3.transformer.VideoEncoderSettings
import java.io.File
import java.io.IOException
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext

@androidx.annotation.OptIn(UnstableApi::class)
internal suspend fun uprightVideoAt(context: Context, file: File) {
  val retriever = MediaMetadataRetriever()
  val bitrate: Int
  try {
    retriever.setDataSource(file.absolutePath)
    val rotation = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION)?.toInt()
      ?: throw IOException("the video has no rotation metadata")
    if (rotation == 0) return
    bitrate = retriever.extractMetadata(MediaMetadataRetriever.METADATA_KEY_BITRATE)?.toIntOrNull()?.takeIf { it > 0 } ?: 8_000_000
  } finally {
    retriever.release()
  }
  val output = File.createTempFile("upright-", ".mp4", file.absoluteFile.parentFile)
  try {
    if (!output.delete()) throw IOException("cannot prepare video output")
    withContext(Dispatchers.Main.immediate) {
      suspendCancellableCoroutine<Unit> { continuation ->
        val encoder = DefaultEncoderFactory.Builder(context)
          .setRequestedVideoEncoderSettings(VideoEncoderSettings.Builder().setBitrate(bitrate).build()).build()
        val transformer = Transformer.Builder(context).setEncoderFactory(encoder).setPortraitEncodingEnabled(true)
          .addListener(object : Transformer.Listener {
            override fun onCompleted(composition: Composition, exportResult: ExportResult) {
              if (continuation.isActive) continuation.resume(Unit)
            }

            override fun onError(composition: Composition, exportResult: ExportResult, exportException: ExportException) {
              if (continuation.isActive) continuation.resumeWithException(IOException("the video cannot be encoded", exportException))
            }
          }).build()
        continuation.invokeOnCancellation {
          if (Looper.myLooper() == Looper.getMainLooper()) transformer.cancel()
          else Handler(Looper.getMainLooper()).post { transformer.cancel() }
        }
        transformer.start(MediaItem.fromUri(Uri.fromFile(file)), output.absolutePath)
      }
    }
    if (!output.renameTo(file)) throw IOException("cannot replace the video")
  } finally {
    output.delete()
  }
}
