package com.noma4i.medialayer.ui

import androidx.media3.common.MimeTypes
import androidx.media3.exoplayer.mediacodec.MediaCodecUtil
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch

internal class CodecWarmup(
  private val dispatcher: CoroutineDispatcher = Dispatchers.IO,
  private val lookup: (String) -> Unit = ::fillDecoderCache
) {
  private val started = AtomicBoolean(false)
  private val scope = CoroutineScope(SupervisorJob() + dispatcher)

  fun start() {
    if (!started.compareAndSet(false, true)) return
    scope.launch { MIMES.forEach(lookup) }
  }

  companion object {
    val process = CodecWarmup()

    internal val MIMES = listOf(MimeTypes.VIDEO_H264, MimeTypes.VIDEO_H265, MimeTypes.AUDIO_AAC)
  }
}

private fun fillDecoderCache(mime: String) {
  try {
    MediaCodecUtil.getDecoderInfos(mime, false, false)
  } catch (_: MediaCodecUtil.DecoderQueryException) {
    // The playback path hits the same query and reports it there.
  }
}
