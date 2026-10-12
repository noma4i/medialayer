package com.noma4i.medialayer.ui

import androidx.media3.exoplayer.DefaultLoadControl
import androidx.media3.exoplayer.LoadControl
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

class ReadaheadLoadControl internal constructor(private val delegate: DefaultLoadControl) : LoadControl by delegate {
  @Volatile private var readaheadUs = Long.MAX_VALUE

  fun setReadaheadMs(ms: Long) {
    readaheadUs = ms * 1_000
  }

  override fun shouldContinueLoading(parameters: LoadControl.Parameters): Boolean =
    parameters.bufferedDurationUs < readaheadUs && delegate.shouldContinueLoading(parameters)
}
