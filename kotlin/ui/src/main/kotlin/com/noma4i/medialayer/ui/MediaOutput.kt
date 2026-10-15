package com.noma4i.medialayer.ui

import androidx.media3.ui.compose.SURFACE_TYPE_SURFACE_VIEW
import androidx.media3.ui.compose.SURFACE_TYPE_TEXTURE_VIEW

enum class MediaOutput { Texture, Window }

internal fun MediaOutput.contentFrameType(): Int = when (this) {
  MediaOutput.Texture -> SURFACE_TYPE_TEXTURE_VIEW
  MediaOutput.Window -> SURFACE_TYPE_SURFACE_VIEW
}
