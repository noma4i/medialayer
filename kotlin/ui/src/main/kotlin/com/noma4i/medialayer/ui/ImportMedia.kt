package com.noma4i.medialayer.ui

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.importExtension
import android.os.ParcelFileDescriptor
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.nio.ByteBuffer
import java.nio.channels.Channels
import java.nio.channels.WritableByteChannel
import java.io.OutputStream
import java.util.UUID
import kotlin.coroutines.CoroutineContext
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.runInterruptible
import kotlinx.coroutines.withContext

data class ImportProgress(val bytesCopied: Long, val totalBytes: Long?)

/** Progress runs on the copy thread. Hop to the main thread to update UI. */
suspend fun MediaLayer.importMedia(uri: Uri, directory: File? = null, progress: ((ImportProgress) -> Unit)? = null): PreparedMedia {
  val context = checkNotNull(binding) { "the layer was not opened with MediaLayer.open(context)" }.factory.appContext
  return importMediaAt(context, uri, directory ?: context.cacheDir, progress)
}

internal suspend fun importMediaAt(context: Context, uri: Uri, directory: File = context.cacheDir, progress: ((ImportProgress) -> Unit)? = null, openOutput: (File) -> OutputStream = { it.outputStream() }): PreparedMedia {
  var output: File? = null
  try {
    return withContext(prepareDispatcher) {
      val copying = currentCoroutineContext()
      runInterruptible {
        val resolver = context.contentResolver
        val metadata = resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { cursor ->
          if (cursor.moveToFirst()) {
            val nameColumn = cursor.getColumnIndex(OpenableColumns.DISPLAY_NAME)
            val sizeColumn = cursor.getColumnIndex(OpenableColumns.SIZE)
            (if (nameColumn >= 0) cursor.getString(nameColumn) else null) to
              (if (sizeColumn >= 0 && !cursor.isNull(sizeColumn)) cursor.getLong(sizeColumn) else null)
          } else null
        }
        val extension = importExtension(metadata?.first, resolver.getType(uri))
        val suffix = if (extension.isEmpty()) "" else ".$extension"
        if (!directory.isDirectory && !directory.mkdirs() && !directory.isDirectory) throw IOException("the destination directory cannot be created")
        val file = File(directory, "imported-${UUID.randomUUID()}$suffix")
        output = file
        val descriptor = try {
          resolver.openFileDescriptor(uri, "r")
        } catch (_: IOException) {
          null
        }
        val totalBytes = descriptor?.statSize?.takeIf { it >= 0 } ?: metadata?.second
        if (descriptor != null && descriptor.statSize >= 0) {
          copyByChannel(descriptor, { openOutput(file) }, copying, totalBytes, progress)
        } else {
          descriptor?.close()
          val input = resolver.openInputStream(uri) ?: throw MediaException.Fetch("the source cannot be opened")
          input.use { source ->
            openOutput(file).use { destination ->
              val buffer = ByteArray(64 * 1024)
              var copied = 0L
              copying.ensureActive()
              progress?.invoke(ImportProgress(copied, totalBytes))
              while (true) {
                copying.ensureActive()
                val count = source.read(buffer)
                copying.ensureActive()
                if (count < 0) break
                destination.write(buffer, 0, count)
                copying.ensureActive()
                copied += count
                progress?.invoke(ImportProgress(copied, totalBytes))
              }
            }
          }
        }
        copying.ensureActive()
        PreparedMedia(file, true)
      }
    }
  } catch (error: Throwable) {
    withContext(NonCancellable + prepareDispatcher) { output?.let(::deleteMediaFile) }
    if (error is CancellationException || error is MediaException) throw error
    throw MediaException.Fetch("the source cannot be imported: ${error.message}")
  }
}

private const val CHANNEL_CHUNK = 1L shl 20

private fun copyByChannel(descriptor: ParcelFileDescriptor, openOutput: () -> OutputStream, copying: CoroutineContext, totalBytes: Long?, progress: ((ImportProgress) -> Unit)?) {
  ParcelFileDescriptor.AutoCloseInputStream(descriptor).use { input ->
    val source = input.channel
    openOutput().use { stream ->
      val target: WritableByteChannel = (stream as? FileOutputStream)?.channel ?: Channels.newChannel(stream)
      val buffer = ByteBuffer.allocate(64 * 1024)
      var position = 0L
      copying.ensureActive()
      progress?.invoke(ImportProgress(position, totalBytes))
      while (true) {
        copying.ensureActive()
        val moved = source.transferTo(position, CHANNEL_CHUNK, target)
        if (moved > 0) {
          copying.ensureActive()
          position += moved
          progress?.invoke(ImportProgress(position, totalBytes))
          continue
        }
        buffer.clear()
        val count = source.read(buffer, position)
        if (count < 0) break
        buffer.flip()
        while (buffer.hasRemaining()) target.write(buffer)
        copying.ensureActive()
        position += count
        progress?.invoke(ImportProgress(position, totalBytes))
      }
    }
  }
}
