package com.noma4i.medialayer.ui

import android.graphics.SurfaceTexture
import android.os.Handler
import android.os.Looper
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.TextureView
import androidx.media3.common.ForwardingPlayer
import androidx.media3.common.Player
import androidx.media3.common.Timeline
import androidx.media3.common.Tracks
import androidx.media3.common.VideoSize
import androidx.media3.exoplayer.ExoPlayer
import java.util.concurrent.CountDownLatch

internal class LooperPlayer(private val exo: ExoPlayer) : ForwardingPlayer(exo) {
  private class Snapshot(exo: ExoPlayer) {
    val videoSize: VideoSize = exo.videoSize
    val tracks: Tracks = exo.currentTracks
    val timeline: Timeline = exo.currentTimeline
    val periodIndex = exo.currentPeriodIndex
    val mediaItemIndex = exo.currentMediaItemIndex
    val playbackState = exo.playbackState
    val commands: Player.Commands = exo.availableCommands
  }

  private interface Binding {
    fun close()
  }

  private val handler = Handler(exo.applicationLooper)

  @Volatile private var snapshot = Snapshot(exo)
  private var binding: Binding? = null

  init {
    exo.addListener(object : Player.Listener {
      override fun onEvents(player: Player, events: Player.Events) {
        snapshot = Snapshot(exo)
      }
    })
  }

  override fun getVideoSize(): VideoSize = snapshot.videoSize

  override fun getCurrentTracks(): Tracks = snapshot.tracks

  override fun getCurrentTimeline(): Timeline = snapshot.timeline

  override fun getCurrentPeriodIndex(): Int = snapshot.periodIndex

  override fun getCurrentMediaItemIndex(): Int = snapshot.mediaItemIndex

  override fun getPlaybackState(): Int = snapshot.playbackState

  override fun getAvailableCommands(): Player.Commands = snapshot.commands

  override fun isCommandAvailable(command: Int): Boolean = snapshot.commands.contains(command)

  override fun addListener(listener: Player.Listener) {
    handler.post { super.addListener(listener) }
  }

  override fun removeListener(listener: Player.Listener) {
    handler.post { super.removeListener(listener) }
  }

  override fun setVideoSurface(surface: Surface?) {
    closeBinding()
    handler.post { super.setVideoSurface(surface) }
  }

  override fun clearVideoSurface() {
    closeBinding()
    handler.post { super.clearVideoSurface() }
  }

  override fun clearVideoSurface(surface: Surface?) {
    handler.post { super.clearVideoSurface(surface) }
  }

  override fun setVideoSurfaceHolder(surfaceHolder: SurfaceHolder?) {
    closeBinding()
    if (surfaceHolder == null) return clearVideoSurface()
    binding = HolderBinding(surfaceHolder)
  }

  override fun clearVideoSurfaceHolder(surfaceHolder: SurfaceHolder?) {
    if (surfaceHolder == null || (binding as? HolderBinding)?.holder === surfaceHolder) closeBinding()
  }

  override fun setVideoSurfaceView(surfaceView: SurfaceView?) = setVideoSurfaceHolder(surfaceView?.holder)

  override fun clearVideoSurfaceView(surfaceView: SurfaceView?) = clearVideoSurfaceHolder(surfaceView?.holder)

  override fun setVideoTextureView(textureView: TextureView?) {
    closeBinding()
    if (textureView == null) return clearVideoSurface()
    binding = TextureBinding(textureView)
  }

  override fun clearVideoTextureView(textureView: TextureView?) {
    if (textureView == null || (binding as? TextureBinding)?.view === textureView) closeBinding()
  }

  private fun closeBinding() {
    binding?.close()
    binding = null
  }

  private fun clearOnPlayer(surface: Surface, release: Boolean, extra: () -> Unit = {}) {
    val posted = handler.post {
      try {
        exo.clearVideoSurface(surface)
      } finally {
        if (release) surface.release()
        extra()
      }
    }
    if (!posted) {
      if (release) surface.release()
      extra()
    }
  }

  private fun clearOnPlayerAndWait(surface: Surface) {
    if (Looper.myLooper() == handler.looper) return exo.clearVideoSurface(surface)
    val cleared = CountDownLatch(1)
    val posted = handler.post {
      try {
        exo.clearVideoSurface(surface)
      } finally {
        cleared.countDown()
      }
    }
    if (posted) cleared.await()
  }

  private inner class HolderBinding(val holder: SurfaceHolder) : Binding, SurfaceHolder.Callback {
    private var surface: Surface? = null

    init {
      holder.addCallback(this)
      holder.surface?.takeIf { it.isValid }?.let { show(it) }
    }

    private fun show(shown: Surface) {
      surface = shown
      handler.post { exo.setVideoSurface(shown) }
    }

    override fun surfaceCreated(holder: SurfaceHolder) = show(holder.surface)

    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) = Unit

    override fun surfaceDestroyed(holder: SurfaceHolder) {
      val destroyed = surface ?: return
      surface = null
      clearOnPlayerAndWait(destroyed)
    }

    override fun close() {
      holder.removeCallback(this)
      surface?.let { clearOnPlayer(it, release = false) }
      surface = null
    }
  }

  private inner class TextureBinding(val view: TextureView) : Binding, TextureView.SurfaceTextureListener {
    private var surface: Surface? = null

    init {
      view.surfaceTextureListener = this
      view.surfaceTexture?.takeIf { view.isAvailable }?.let { onSurfaceTextureAvailable(it, view.width, view.height) }
    }

    override fun onSurfaceTextureAvailable(texture: SurfaceTexture, width: Int, height: Int) {
      val shown = Surface(texture)
      surface = shown
      handler.post { exo.setVideoSurface(shown) }
    }

    override fun onSurfaceTextureSizeChanged(texture: SurfaceTexture, width: Int, height: Int) = Unit

    override fun onSurfaceTextureDestroyed(texture: SurfaceTexture): Boolean {
      val destroyed = surface ?: return true
      surface = null
      clearOnPlayer(destroyed, release = true) { texture.release() }
      return false
    }

    override fun onSurfaceTextureUpdated(texture: SurfaceTexture) = Unit

    override fun close() {
      if (view.surfaceTextureListener === this) view.surfaceTextureListener = null
      surface?.let { clearOnPlayer(it, release = true) }
      surface = null
    }
  }
}
