package com.noma4i.medialayer.ui

import android.net.Uri
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.isLocalMediaUrl
import java.io.File
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.Dispatchers

/** The file to send. `isNew` is false when the source already fitted and is handed back untouched: the caller owns a file only when `isNew` is true. */
data class PreparedMedia(val file: File, val isNew: Boolean)

internal var prepareDispatcher: CoroutineDispatcher = Dispatchers.IO

internal fun preparableFile(url: String): File {
  val trimmed = url.trim()
  val uri = Uri.parse(trimmed)
  if (!isLocalMediaUrl(trimmed) || uri.scheme != "file") throw MediaException.Fetch("$url is not a file:// source")
  val file = File(checkNotNull(uri.path))
  if (!file.isFile || !file.canRead()) throw MediaException.Fetch("$url is not a readable file")
  return file
}
