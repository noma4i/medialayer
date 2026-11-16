package com.noma4i.medialayer.ui

import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MediaPlace
import com.noma4i.medialayer.MediaState
import com.noma4i.medialayer.PlaceInput
import com.noma4i.medialayer.PlaybackListener
import com.noma4i.medialayer.PlaybackState
import com.noma4i.medialayer.Subscription

internal data class PlaceUpdate(val input: PlaceInput? = null, val seek: Double? = null, val state: MediaState? = null, val closes: Boolean = false) {
  companion object {
    fun merged(held: PlaceUpdate?, next: PlaceUpdate) =
      PlaceUpdate(next.input ?: held?.input, next.seek ?: held?.seek, next.state ?: held?.state, (held?.closes ?: false) || next.closes)
  }
}

internal fun placeInput(
  visible: Boolean = false, playing: Boolean = false, group: String? = null, peeking: Boolean = false,
  muted: Boolean? = null, rate: Float? = null, pinned: Boolean = false, viewToken: ULong? = null, playGeneration: ULong = 0uL
) = PlaceInput(visible, playing, playGeneration, group, peeking, muted, rate, pinned, viewToken)

internal fun Throwable.asMediaException(): MediaException = this as? MediaException ?: MediaException.Unexpected(toString())

/** One mounted place. Calls leave the caller thread through one latest-wins drain; a refused call reaches [failed]. */
internal class PlaceMount(media: MediaLayer, item: MediaItem, private val playback: (PlaybackState) -> Unit = {}, failed: (MediaException) -> Unit) {
  private var place: MediaPlace? = null
  private var subscription: Subscription? = null
  private var closed = false
  private val drain = LatestDrain(PlaceUpdate::merged) { update ->
    if (closed) return@LatestDrain
    if (update.closes) {
      closed = true
      subscription?.close()
      subscription = null
      place?.let { opened ->
        try {
          try { update.seek?.let(opened::seek) } catch (error: MediaException) { failed(error) }
          opened.release()
        } finally { opened.close() }
      }
      place = null
      return@LatestDrain
    }
    try {
      if (place == null) {
        place = media.openPlace(item)
        subscription = place?.subscribePlayback(object : PlaybackListener {
          override fun changed(state: PlaybackState) = playback(state)
          override fun failed(error: MediaException) { if (error !is MediaException.Closed) failed(error) }
        })
      }
      val opened = place ?: return@LatestDrain
      update.input?.let(opened::update)
      update.seek?.let(opened::seek)
      update.state?.let(opened::itemState)
    } catch (error: MediaException) {
      failed(error)
    }
  }

  fun update(input: PlaceInput) = drain.submit(PlaceUpdate(input = input))

  fun seek(fraction: Double) = drain.submit(PlaceUpdate(seek = fraction))

  fun report(state: MediaState) = drain.submit(PlaceUpdate(state = state))

  fun close() = drain.submit(PlaceUpdate(closes = true))
}
