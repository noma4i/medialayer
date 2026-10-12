package com.noma4i.medialayer.ui

import androidx.media3.common.Timeline
import androidx.media3.exoplayer.DefaultLoadControl
import androidx.media3.exoplayer.LoadControl
import androidx.media3.exoplayer.analytics.PlayerId
import androidx.media3.exoplayer.source.MediaSource
import androidx.media3.exoplayer.source.TrackGroupArray
import androidx.media3.exoplayer.trackselection.ExoTrackSelection
import androidx.media3.exoplayer.upstream.Allocator
import androidx.media3.exoplayer.upstream.DefaultAllocator

data class LoadControlConfig(
  val bufferForPlaybackMs: Int = 1_000,
  val bufferForPlaybackAfterRebufferMs: Int = 2_000,
  val backBufferMs: Int = 0
) {
  fun create(allocator: DefaultAllocator): ReadaheadLoadControl {
    val ceilingMs = DefaultLoadControl.DEFAULT_MAX_BUFFER_MS
    val delegate = DefaultLoadControl.Builder()
      .setAllocator(allocator)
      .setBufferDurationsMs(ceilingMs, ceilingMs, bufferForPlaybackMs, bufferForPlaybackAfterRebufferMs)
      .setBackBuffer(backBufferMs, false)
      .build()
    return ReadaheadLoadControl(delegate)
  }
}

class ReadaheadLoadControl internal constructor(private val delegate: DefaultLoadControl) : LoadControl {
  @Volatile private var readaheadUs = Long.MAX_VALUE

  fun setReadaheadMs(ms: Long) {
    readaheadUs = ms * 1_000
  }

  override fun onPrepared(playerId: PlayerId) = delegate.onPrepared(playerId)

  override fun onTracksSelected(parameters: LoadControl.Parameters, trackGroups: TrackGroupArray, trackSelections: Array<ExoTrackSelection?>) =
    delegate.onTracksSelected(parameters, trackGroups, trackSelections)

  override fun onStopped(playerId: PlayerId) = delegate.onStopped(playerId)

  override fun onReleased(playerId: PlayerId) = delegate.onReleased(playerId)

  override fun getAllocator(playerId: PlayerId): Allocator = delegate.getAllocator(playerId)

  override fun getBackBufferDurationUs(playerId: PlayerId): Long = delegate.getBackBufferDurationUs(playerId)

  override fun retainBackBufferFromKeyframe(playerId: PlayerId): Boolean = delegate.retainBackBufferFromKeyframe(playerId)

  override fun shouldContinuePreloading(playerId: PlayerId, timeline: Timeline, mediaPeriodId: MediaSource.MediaPeriodId, bufferedDurationUs: Long): Boolean =
    delegate.shouldContinuePreloading(playerId, timeline, mediaPeriodId, bufferedDurationUs)

  override fun shouldContinueLoading(parameters: LoadControl.Parameters): Boolean =
    parameters.bufferedDurationUs < readaheadUs && delegate.shouldContinueLoading(parameters)

  override fun shouldStartPlayback(parameters: LoadControl.Parameters): Boolean = delegate.shouldStartPlayback(parameters)
}
