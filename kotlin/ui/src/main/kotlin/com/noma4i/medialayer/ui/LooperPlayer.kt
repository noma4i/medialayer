package com.noma4i.medialayer.ui

import android.os.Handler
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

  private val handler = Handler(exo.applicationLooper)

  @Volatile private var snapshot = Snapshot(exo)

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
    handler.post { super.setVideoSurface(surface) }
  }

  override fun clearVideoSurface() {
    handler.post { super.clearVideoSurface() }
  }

  override fun clearVideoSurface(surface: Surface?) {
    handler.post { super.clearVideoSurface(surface) }
  }

  override fun setVideoSurfaceHolder(surfaceHolder: SurfaceHolder?) {
    handler.post { super.setVideoSurfaceHolder(surfaceHolder) }
  }

  override fun clearVideoSurfaceHolder(surfaceHolder: SurfaceHolder?) {
    handler.post { super.clearVideoSurfaceHolder(surfaceHolder) }
  }

  override fun setVideoSurfaceView(surfaceView: SurfaceView?) {
    handler.post { super.setVideoSurfaceView(surfaceView) }
  }

  override fun clearVideoSurfaceView(surfaceView: SurfaceView?) {
    handler.post { super.clearVideoSurfaceView(surfaceView) }
  }

  override fun setVideoTextureView(textureView: TextureView?) {
    handler.post { super.setVideoTextureView(textureView) }
  }

  override fun clearVideoTextureView(textureView: TextureView?) {
    handler.post { super.clearVideoTextureView(textureView) }
  }
}
