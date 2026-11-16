package com.noma4i.medialayer.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.remember
import androidx.compose.runtime.mutableStateOf
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaTone
import com.noma4i.medialayer.ToneUpdate
import com.noma4i.medialayer.toItem
import com.noma4i.medialayer.toneFlow

data class MediaToneState(val tone: MediaTone? = null, val error: MediaException? = null)

private class FollowedSource {
  var item: MediaItem? = null
}

/**
 * Follows the tone of [source] at [band] while composed: the state holds the current tone (null while unknown) and the failure of the watch.
 * A null source cancels the watch and clears the value; a new band keeps the value until the new one arrives.
 */
@Composable
fun rememberMediaTone(source: MediaSource?, band: Float): State<MediaToneState> {
  val media = LocalMediaEnvironment.current.media
  val item = source?.toItem()
  val state = remember { mutableStateOf(MediaToneState()) }
  val followed = remember { FollowedSource() }
  LaunchedEffect(media, item, band) {
    if (followed.item != item) state.value = MediaToneState()
    followed.item = item
    if (item == null) return@LaunchedEffect
    media.toneFlow(item, band).collect { update ->
      state.value = when (update) {
        is ToneUpdate.Value -> MediaToneState(update.tone)
        is ToneUpdate.Failed -> state.value.copy(error = update.error)
      }
    }
  }
  return state
}
