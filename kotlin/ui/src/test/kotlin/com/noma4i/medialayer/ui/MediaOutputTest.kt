package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.Outline
import android.graphics.Rect
import android.os.Build
import android.view.SurfaceView
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.ui.compose.SURFACE_TYPE_SURFACE_VIEW
import androidx.media3.ui.compose.SURFACE_TYPE_TEXTURE_VIEW
import androidx.test.core.app.ApplicationProvider
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

@RunWith(MediaLayerTestRunner::class)
@Config(sdk = [30, 31, 34])
class MediaOutputTest {
  @Test
  fun inv_output_01_window_requires_android_14() {
    assertEquals(SURFACE_TYPE_TEXTURE_VIEW, MediaOutput.Texture.contentFrameType())
    val expected = if (Build.VERSION.SDK_INT >= 34) SURFACE_TYPE_SURFACE_VIEW else SURFACE_TYPE_TEXTURE_VIEW
    assertEquals(expected, MediaOutput.Window.contentFrameType())
  }

  @Test
  fun inv_output_03_fading_video_uses_texture() {
    for (alpha in listOf(0f, 0.5f, 0.99f)) {
      assertEquals(SURFACE_TYPE_TEXTURE_VIEW, MediaOutput.Window.contentFrameType(alpha))
    }
  }

  @Test
  @Config(sdk = [34])
  fun inv_output_04_window_configures_a_resizable_public_outline() {
    val context = ApplicationProvider.getApplicationContext<Context>()
    val player = ExoPlayer.Builder(context).build()
    val measured = VideoOutputPlayer(player, null, 16f)
    val view = SurfaceView(context)
    try {
      measured.setVideoSurfaceView(view)
      assertTrue(view.clipToOutline)
      for (size in listOf(100, 200)) {
        view.layout(0, 0, size, size * 2)
        val outline = Outline()
        view.outlineProvider.getOutline(view, outline)
        val bounds = Rect()
        assertTrue(outline.getRect(bounds))
        assertEquals(Rect(0, 0, size, size * 2), bounds)
        assertEquals(16f, outline.radius, 0f)
      }
    } finally {
      player.release()
    }
  }

  @Test
  fun inv_output_02_surface_bindings_count_calls_and_distinct_views() {
    val context = ApplicationProvider.getApplicationContext<Context>()
    val player = ExoPlayer.Builder(context).build()
    val counters = VideoOutputCounters()
    val measured = VideoOutputPlayer(player, counters)
    val first = SurfaceView(context)
    val second = SurfaceView(context)
    try {
      measured.setVideoSurfaceView(first)
      measured.setVideoSurfaceView(first)
      measured.setVideoSurfaceView(second)
      measured.clearVideoSurfaceView(second)
      val counts = counters.snapshot()
      assertEquals(3L, counts["setVideoSurfaceView"])
      assertEquals(2L, counts["surfaceCreate"])
      assertEquals(1L, counts["clearVideoSurfaceView"])
      assertEquals(0L, counts["liveSurfaces"])
      assertEquals(null, counts["setVideoTextureView"])
    } finally {
      player.release()
    }
  }
}
