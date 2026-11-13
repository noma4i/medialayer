package com.noma4i.medialayer.ui

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.importExtension
import android.os.ParcelFileDescriptor
import java.io.File
import java.io.FileInputStream
import java.io.IOException
import java.nio.ByteBuffer
import java.nio.channels.Channels
import java.io.OutputStream
import java.util.UUID
import kotlin.coroutines.CoroutineContext
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.runInterruptible
import kotlinx.coroutines.withContext

suspend fun MediaLayer.importMedia(uri: Uri): PreparedMedia {
  val context = checkNotNull(binding) { "the layer was not opened with MediaLayer.open(context)" }.factory.appContext
  return importMediaAt(context, uri)
}

internal suspend fun importMediaAt(context: Context, uri: Uri, openOutput: (File) -> OutputStream = { it.outputStream() }): PreparedMedia {
  var output: File? = null
  try {
    return withContext(prepareDispatcher) {
      val copying = currentCoroutineContext()
      runInterruptible {
        val resolver = context.contentResolver
        val name = resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
          if (cursor.moveToFirst()) cursor.getString(0) else null
        }
        val extension = importExtension(name, resolver.getType(uri))
        val suffix = if (extension.isEmpty()) "" else ".$extension"
        val file = File(context.cacheDir, "imported-${UUID.randomUUID()}$suffix")
        output = file
        val descriptor = try {
          resolver.openFileDescriptor(uri, "r")
        } catch (_: IOException) {
          null
        }
        if (descriptor != null && descriptor.statSize >= 0) {
          descriptor.use { copyByChannel(it, openOutput(file), copying) }
        } else {
          descriptor?.close()
          val input = resolver.openInputStream(uri) ?: throw MediaException.Fetch("the source cannot be opened")
          input.use { source ->
            openOutput(file).use { destination ->
              val buffer = ByteArray(64 * 1024)
              while (true) {
                copying.ensureActive()
                val count = source.read(buffer)
                copying.ensureActive()
                if (count < 0) break
                destination.write(buffer, 0, count)
              }
            }
          }
        }
        copying.ensureActive()
        PreparedMedia(file, true)
      }
    }
  } catch (error: Throwable) {
    withContext(NonCancellable + prepareDispatcher) { output?.delete() }
    if (error is CancellationException || error is MediaException) throw error
    throw MediaException.Fetch("the source cannot be imported: ${error.message}")
  }
}

private const val CHANNEL_CHUNK = 1L shl 20

private fun copyByChannel(descriptor: ParcelFileDescriptor, destination: OutputStream, copying: CoroutineContext) {
  FileInputStream(descriptor.fileDescriptor).channel.use { source ->
    destination.use { stream ->
      val target = Channels.newChannel(stream)
      val buffer = ByteBuffer.allocate(64 * 1024)
      var position = 0L
      while (true) {
        copying.ensureActive()
        val moved = source.transferTo(position, CHANNEL_CHUNK, target)
        if (moved > 0) {
          position += moved
          continue
        }
        buffer.clear()
        val count = source.read(buffer, position)
        if (count < 0) break
        buffer.flip()
        target.write(buffer)
        position += count
      }
    }
  }
}
