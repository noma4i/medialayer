package com.noma4i.medialayer.ui

import androidx.media3.common.Player

internal class AudioDemand {
  private val audible = HashSet<UInt>()
  var recordings = 0
    private set
  val recording: Boolean get() = recordings > 0

  fun setAudible(slot: UInt, value: Boolean): Boolean = if (value) audible.add(slot) else audible.remove(slot)

  fun setRecording(value: Boolean) {
    recordings = if (value) recordings + 1 else (recordings - 1).coerceAtLeast(0)
  }

  fun target(focus: MediaAudioFocus): FocusKind? = when {
    recording -> FocusKind.Recording
    audible.isNotEmpty() && focus == MediaAudioFocus.Duck -> FocusKind.Duck
    else -> null
  }
}

internal fun isAudible(volume: Float, playWhenReady: Boolean, playbackState: Int, hasItem: Boolean): Boolean =
  volume > 0f && playWhenReady && hasItem && playbackState != Player.STATE_ENDED
