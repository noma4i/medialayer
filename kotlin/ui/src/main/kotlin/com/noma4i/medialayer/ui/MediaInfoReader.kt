package com.noma4i.medialayer.ui

import android.content.Context
import android.content.pm.PackageManager
import android.content.res.Resources
import android.media.MediaMetadataRetriever
import android.net.Uri
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaInfo
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.displayFrameSize
import com.noma4i.medialayer.isLocalMediaUrl
import com.noma4i.medialayer.normalizeItem
import com.noma4i.medialayer.toItem
import java.io.File
import java.io.IOException
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.withContext

/**
 * Length and frame of a local video or audio file, read without a player. A remote source has no metadata without a download and gives `null`.
 * A file that is not media gives `null`, a missing or unreadable file throws `MediaException.Fetch`.
 */
suspend fun MediaLayer.mediaInfo(source: MediaSource): MediaInfo? {
  return withContext(mediaInfoDispatcher) {
    val url = normalizeItem(source.toItem()).url
    if (!isLocalMediaUrl(url)) return@withContext null
    val context = checkNotNull(binding) { "the layer was not opened with MediaLayer.open(context)" }.factory.appContext
    readMediaInfo(context, url)
  }
}

internal var mediaInfoDispatcher: CoroutineDispatcher = Dispatchers.IO

internal suspend fun readMediaInfo(context: Context, url: String): MediaInfo? {
  val retriever = MediaMetadataRetriever()
  try {
    try {
      attach(retriever, context, url)
    } catch (_: IOException) {
      throw MediaException.Fetch("$url is not a readable file")
    } catch (_: RuntimeException) {
      return null
    }
    currentCoroutineContext().ensureActive()
    fun field(key: Int) = retriever.extractMetadata(key)
    val duration = field(MediaMetadataRetriever.METADATA_KEY_DURATION)?.toLongOrNull()?.takeIf { it > 0 }?.toULong()
    val width = field(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH)?.toUIntOrNull()
    val height = field(MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT)?.toUIntOrNull()
    val rotation = field(MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION)?.toIntOrNull() ?: 0
    currentCoroutineContext().ensureActive()
    val frame = if (width != null && height != null) displayFrameSize(width, height, rotation) else null
    return if (duration == null && frame == null) null else MediaInfo(duration, frame)
  } finally {
    retriever.release()
  }
}

private fun attach(retriever: MediaMetadataRetriever, context: Context, url: String) {
  val uri = Uri.parse(url)
  when (uri.scheme) {
    "file" -> {
      val file = File(checkNotNull(uri.path))
      if (!file.isFile || !file.canRead()) throw IOException("missing")
      retriever.setDataSource(file.path)
    }
    "android.resource" -> {
      val id = uri.lastPathSegment?.toIntOrNull() ?: throw IOException("no resource id")
      val descriptor = try {
        context.packageManager.getResourcesForApplication(uri.host ?: context.packageName).openRawResourceFd(id)
      } catch (error: PackageManager.NameNotFoundException) {
        throw IOException(error)
      } catch (error: Resources.NotFoundException) {
        throw IOException(error)
      }
      descriptor.use { retriever.setDataSource(it.fileDescriptor, it.startOffset, it.length) }
    }
    else -> context.assets.openFd(checkNotNull(uri.path).trimStart('/')).use { retriever.setDataSource(it.fileDescriptor, it.startOffset, it.length) }
  }
}
