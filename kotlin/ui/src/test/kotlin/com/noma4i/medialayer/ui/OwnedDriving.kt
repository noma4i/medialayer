package com.noma4i.medialayer.ui

import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MediaPlace
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaState
import com.noma4i.medialayer.MediaWindow
import com.noma4i.medialayer.PlaceInput
import com.noma4i.medialayer.toItem
import java.util.WeakHashMap

/** Test driver: per-key calls over the owned places and windows of the library. */
internal class OwnedDriving(private val media: MediaLayer) {
  private val places = HashMap<String, MediaPlace>()
  private val inputs = HashMap<String, PlaceInput>()
  private var video: MediaWindow? = null
  private val images = HashMap<String, MediaWindow>()

  @Synchronized
  fun edit(key: String, item: MediaItem? = null, change: (PlaceInput) -> PlaceInput) {
    val place = places.getOrPut(key) { media.openPlace(item ?: MediaItem(com.noma4i.medialayer.Kind.VIDEO, key, emptyMap(), null, null, strategy = com.noma4i.medialayer.CacheStrategy.WINDOW)) }
    val input = change(inputs[key] ?: PlaceInput(false, false, null, false, null, null, false, null))
    inputs[key] = input
    place.update(input)
  }

  @Synchronized fun place(key: String): MediaPlace? = places[key]

  @Synchronized
  fun drop(key: String) {
    places.remove(key)?.release()
    inputs.remove(key)
  }

  @Synchronized fun videoWindow(): MediaWindow = video ?: media.openVideoWindow().also { video = it }

  @Synchronized fun imageWindow(name: String): MediaWindow = images.getOrPut(name) { media.openImageWindow(name) }
}

private val drivers = WeakHashMap<MediaLayer, OwnedDriving>()

internal val MediaLayer.driving: OwnedDriving get() = synchronized(drivers) { drivers.getOrPut(this) { OwnedDriving(this) } }

internal fun MediaLayer.visible(source: MediaItem, visible: Boolean) {
  if (visible) driving.edit(source.url, source) { it.copy(visible = true) } else driving.drop(source.url)
}

internal fun MediaLayer.setPlaying(key: String, playing: Boolean, groups: List<String?> = emptyList()) =
  driving.edit(key) { it.copy(playing = playing, group = groups.filterNotNull().firstOrNull()) }

internal fun MediaLayer.setPeeking(key: String, peeking: Boolean) = driving.edit(key) { it.copy(peeking = peeking) }

internal fun MediaLayer.setMuted(key: String, muted: Boolean) = driving.edit(key) { it.copy(muted = muted) }

internal fun MediaLayer.setRate(key: String, rate: Float) = driving.edit(key) { it.copy(rate = rate) }

internal fun MediaLayer.attachView(key: String, token: ULong) = driving.edit(key) { it.copy(viewToken = token) }

internal fun MediaLayer.detachView(key: String) = driving.edit(key) { it.copy(viewToken = null) }

internal fun MediaLayer.seek(key: String, fraction: Double) {
  driving.edit(key) { it }
  driving.place(key)?.seek(fraction)
}

internal fun MediaLayer.itemState(key: String, state: MediaState) {
  driving.edit(key) { it }
  driving.place(key)?.itemState(state)
}

internal fun MediaLayer.replaceWindow(active: UInt, sources: List<MediaItem?>) = driving.videoWindow().replace(active, sources)

internal fun MediaLayer.moveWindow(active: UInt) = driving.videoWindow().moveTo(active)

internal fun MediaLayer.replaceImageWindow(name: String, active: Int, pages: List<List<MediaSource>>) =
  driving.imageWindow(name).replacePages(active.coerceAtLeast(0).toUInt(), pages.map { page -> page.map { source -> source.toItem() } })
