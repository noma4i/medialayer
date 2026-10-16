package com.noma4i.medialayer.ui

import androidx.media3.common.Player

internal class AudioDemand {
  private val audible = HashSet<UInt>()
  var recording = false
    private set

  fun setAudible(slot: UInt, value: Boolean) {
    if (value) audible.add(slot) else audible.remove(slot)
  }

  fun setRecording(value: Boolean) {
    recording = value
  }

  fun target(focus: MediaAudioFocus): FocusKind? = when {
    recording -> FocusKind.Recording
    audible.isNotEmpty() && focus == MediaAudioFocus.Duck -> FocusKind.Duck
    else -> null
  }
}

internal fun isAudible(volume: Float, playWhenReady: Boolean, playbackState: Int, hasItem: Boolean): Boolean =
  volume > 0f && playWhenReady && hasItem && playbackState != Player.STATE_ENDED
