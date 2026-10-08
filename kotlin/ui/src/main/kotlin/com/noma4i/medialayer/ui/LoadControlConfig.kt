package com.noma4i.medialayer.ui

import androidx.media3.exoplayer.DefaultLoadControl
import androidx.media3.exoplayer.upstream.DefaultAllocator

data class LoadControlConfig(
  val minBufferMs: Int = 5_000,
  val maxBufferMs: Int = 10_000,
  val bufferForPlaybackMs: Int = 1_000,
  val bufferForPlaybackAfterRebufferMs: Int = 2_000,
  val backBufferMs: Int = 0
) {
  fun create(allocator: DefaultAllocator): DefaultLoadControl =
    DefaultLoadControl.Builder()
      .setAllocator(allocator)
      .setBufferDurationsMs(minBufferMs, maxBufferMs, bufferForPlaybackMs, bufferForPlaybackAfterRebufferMs)
      .setBackBuffer(backBufferMs, false)
      .build()
}
