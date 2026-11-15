package com.noma4i.medialayer.ui

import android.os.Trace
import android.view.TextureView
import androidx.media3.common.ForwardingPlayer
import androidx.media3.common.Player
import java.util.WeakHashMap

class VideoOutputCounters {
  private val totals = mutableMapOf<String, Long>()
  private val attachments = mutableMapOf<UInt, Pair<String?, ULong>>()
  private val textures = WeakHashMap<TextureView, Unit>()

  @Synchronized
  internal fun count(event: String) {
    totals[event] = (totals[event] ?: 0) + 1
    onTestChange?.invoke()
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
  fun snapshot(): Map<String, Long> = totals.toMap() + mapOf(
    "livePlayers" to ((totals["create"] ?: 0) - (totals["release"] ?: 0)),
    "liveTextures" to textures.keys.count { it.isAttachedToWindow }.toLong()
  )
}

internal class VideoOutputPlayer(
  player: Player,
  private val counters: VideoOutputCounters?
) : ForwardingPlayer(player) {
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
