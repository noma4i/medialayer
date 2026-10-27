package com.noma4i.medialayer.ui

import android.content.Context
import android.net.Uri
import com.noma4i.medialayer.MediaException
import java.io.InputStream

internal fun isBundledUrl(url: String): Boolean = url.startsWith("android.resource://") || url.startsWith("asset:///")

/** Opens the app resource or asset behind [url]. Call it where blocking I/O is allowed: the fetcher runs on `Dispatchers.IO`. */
internal fun openBundled(context: Context, url: String): InputStream {
  val uri = Uri.parse(url)
  try {
    if (uri.scheme == "asset") return context.assets.open(checkNotNull(uri.path).trimStart('/'))
    val id = checkNotNull(uri.lastPathSegment?.toIntOrNull()) { "no resource id" }
    val host = uri.host
    val resources = if (host == null || host == context.packageName) context.resources else context.packageManager.getResourcesForApplication(host)
    return resources.openRawResource(id)
  } catch (error: Exception) {
    throw MediaException.Fetch("$url is not a readable resource")
  }
}
