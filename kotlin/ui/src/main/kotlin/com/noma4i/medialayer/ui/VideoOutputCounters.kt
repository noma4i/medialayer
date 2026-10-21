package com.noma4i.medialayer.ui

import android.graphics.Outline
import android.os.Build
import android.os.Trace
import android.view.SurfaceView
import android.view.TextureView
import android.view.View
import android.view.ViewOutlineProvider
import androidx.media3.common.ForwardingPlayer
import androidx.media3.common.Player
import java.util.WeakHashMap

class VideoOutputCounters {
  private val totals = mutableMapOf<String, Long>()
  private val attachments = mutableMapOf<UInt, Pair<String?, ULong>>()
  private val textures = WeakHashMap<TextureView, Unit>()
  private val surfaces = WeakHashMap<SurfaceView, Unit>()

  @Synchronized
  internal fun count(event: String) {
    totals[event] = (totals[event] ?: 0) + 1
  }

  @Synchronized
  internal fun attach(slot: UInt, key: String?, token: ULong) {
    count("attach")
    val next = key to token
    if (attachments.put(slot, next) == next) count("repeatAttach")
  }

  @Synchronized
  internal fun detach(slot: UInt) {
    count("detach")
    attachments.remove(slot)
  }

  @Synchronized
  internal fun texture(view: TextureView?) {
    count("setVideoTextureView")
    if (view != null && textures.put(view, Unit) == null) count("textureCreate")
  }

  @Synchronized
  internal fun surface(view: SurfaceView?) {
    count("setVideoSurfaceView")
    if (view != null && surfaces.put(view, Unit) == null) count("surfaceCreate")
  }

  @Synchronized
  fun snapshot(): Map<String, Long> = totals.toMap() + mapOf(
    "livePlayers" to ((totals["create"] ?: 0) - (totals["release"] ?: 0)),
    "liveTextures" to textures.keys.count { it.isAttachedToWindow }.toLong(),
    "liveSurfaces" to surfaces.keys.count { it.isAttachedToWindow }.toLong()
  )
}

internal class VideoOutputPlayer(
  player: Player,
  private val counters: VideoOutputCounters?,
  private val cornerRadiusPx: Float = 0f
) : ForwardingPlayer(player) {
  private val outline = object : ViewOutlineProvider() {
    override fun getOutline(view: View, outline: Outline) {
      outline.setRoundRect(0, 0, view.width, view.height, cornerRadiusPx)
    }
  }

  override fun setVideoSurfaceView(surfaceView: SurfaceView?) {
    Trace.beginSection("MediaLayer.setVideoSurfaceView")
    try {
      surfaceView?.let {
        it.outlineProvider = outline
        it.clipToOutline = cornerRadiusPx > 0f
        if (Build.VERSION.SDK_INT >= 34) {
          it.setSurfaceLifecycle(SurfaceView.SURFACE_LIFECYCLE_FOLLOWS_ATTACHMENT)
        }
      }
      counters?.surface(surfaceView)
      super.setVideoSurfaceView(surfaceView)
    } finally {
      Trace.endSection()
    }
  }

  override fun clearVideoSurfaceView(surfaceView: SurfaceView?) {
    Trace.beginSection("MediaLayer.clearVideoSurfaceView")
    try {
      counters?.count("clearVideoSurfaceView")
      super.clearVideoSurfaceView(surfaceView)
    } finally {
      Trace.endSection()
    }
  }

  override fun setVideoTextureView(textureView: TextureView?) {
    Trace.beginSection("MediaLayer.setVideoTextureView")
    try {
      counters?.texture(textureView)
      super.setVideoTextureView(textureView)
    } finally {
      Trace.endSection()
    }
  }

  override fun clearVideoTextureView(textureView: TextureView?) {
    Trace.beginSection("MediaLayer.clearVideoTextureView")
    try {
      counters?.count("clearVideoTextureView")
      super.clearVideoTextureView(textureView)
    } finally {
      Trace.endSection()
    }
  }
}
