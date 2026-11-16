package com.noma4i.medialayer.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.PlaybackSuspension

internal class SuspensionHold {
  private var held: PlaybackSuspension? = null
  private var layer: MediaLayer? = null
  private var groups: List<String>? = null

  fun apply(media: MediaLayer?, active: Boolean, allowGroups: List<String>) {
    if (!active || media == null) return release()
    if (held != null && layer === media && groups == allowGroups) return
    val next = try {
      media.suspendPlayback(allowGroups)
    } catch (error: MediaException) {
      groups = null
      Media.recordFailure(error)
      return
    }
    held?.release()
    held = next
    layer = media
    groups = allowGroups
    Media.failure = null
  }

  fun release() {
    held?.release()
    held = null
    layer = null
    groups = null
  }
}

@Composable
fun SuspendMediaPlayback(active: Boolean = true, allowGroups: List<String> = emptyList()) {
  val media = currentMediaLayer()
  val hold = remember { SuspensionHold() }
  DisposableEffect(hold, media, active, allowGroups) {
    hold.apply(media, active, allowGroups)
    onDispose { }
  }
  DisposableEffect(hold) { onDispose { hold.release() } }
}
