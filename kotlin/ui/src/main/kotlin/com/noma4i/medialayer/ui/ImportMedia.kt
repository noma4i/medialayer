package com.noma4i.medialayer.ui

import android.content.Context
import android.net.Uri
import android.provider.OpenableColumns
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.importExtension
import java.io.File
import java.io.OutputStream
import java.util.UUID
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
