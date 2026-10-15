package com.noma4i.medialayer.ui

import androidx.media3.ui.compose.SURFACE_TYPE_SURFACE_VIEW
import androidx.media3.ui.compose.SURFACE_TYPE_TEXTURE_VIEW
import org.junit.Assert.assertEquals
import org.junit.Test

class MediaOutputTest {
  @Test
  fun textureDrawsOnATextureViewAndWindowOnASurfaceView() {
    assertEquals(SURFACE_TYPE_TEXTURE_VIEW, MediaOutput.Texture.contentFrameType())
    assertEquals(SURFACE_TYPE_SURFACE_VIEW, MediaOutput.Window.contentFrameType())
  }
}
