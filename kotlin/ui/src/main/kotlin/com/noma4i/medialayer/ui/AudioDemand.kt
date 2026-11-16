package com.noma4i.medialayer.ui

import androidx.media3.common.Player
import java.util.UUID

internal class AudioDemand {
  private val audible = HashSet<UInt>()
  val recordings = HashSet<UUID>()
  val recording: Boolean get() = recordings.isNotEmpty()

  fun setAudible(slot: UInt, value: Boolean): Boolean = if (value) audible.add(slot) else audible.remove(slot)

  fun setRecording(id: UUID, value: Boolean) {
    if (value) recordings.add(id) else recordings.remove(id)
  }

  fun target(focus: MediaAudioFocus): FocusKind? = when {
    recording -> FocusKind.Recording
    audible.isNotEmpty() && focus == MediaAudioFocus.Duck -> FocusKind.Duck
    else -> null
  }
}

internal fun isAudible(volume: Float, playWhenReady: Boolean, playbackState: Int, hasItem: Boolean): Boolean =
  volume > 0f && playWhenReady && hasItem && playbackState != Player.STATE_ENDED
