package com.noma4i.medialayer.ui

import android.os.Build
import android.os.Trace
import androidx.media3.ui.compose.SURFACE_TYPE_SURFACE_VIEW
import androidx.media3.ui.compose.SURFACE_TYPE_TEXTURE_VIEW

enum class MediaOutput { Texture, Window }

internal fun MediaOutput.contentFrameType(alpha: Float = 1f): Int {
  if (this == MediaOutput.Texture) return SURFACE_TYPE_TEXTURE_VIEW
  val reason = when {
    Build.VERSION.SDK_INT < 34 -> "Texture.api"
    alpha != 1f -> "Texture.alpha"
    else -> "Window"
  }
  Trace.beginSection("MediaLayer.output.$reason")
  Trace.endSection()
  return if (reason == "Window") SURFACE_TYPE_SURFACE_VIEW else SURFACE_TYPE_TEXTURE_VIEW
}
