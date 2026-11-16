package com.noma4i.medialayer.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MediaWholeDeclaration

class MediaWholeHold {
  private var declaration: MediaWholeDeclaration? = null
  private var layer: MediaLayer? = null
  private var held = emptySet<MediaItem>()

  fun declare(media: MediaLayer?, items: List<MediaItem>) {
    if (media == null) return release()
    if (layer !== media) release()
    try {
      val open = declaration ?: media.openWholeDeclaration()
      declaration = open
      layer = media
      val next = items.toSet()
      val fresh = items.filter { it !in held }
      val gone = held - next
      if (gone.isNotEmpty()) open.remove(gone.toList())
      if (fresh.isNotEmpty()) open.add(fresh)
      held = next
      Media.failure = null
    } catch (error: MediaException) {
      Media.recordFailure(error)
    }
  }

  fun add(media: MediaLayer?, items: List<MediaItem>) = declare(media, held.toList() + items)

  fun release() {
    declaration?.let { opened ->
      try { opened.release() } finally { opened.close() }
    }
    declaration = null
    layer = null
    held = emptySet()
  }
}

@Composable
fun DeclareMediaWhole(items: List<MediaItem>) {
  val media = currentMediaLayer()
  val hold = remember { MediaWholeHold() }
  DisposableEffect(hold, media, items) {
    hold.declare(media, items)
    onDispose { }
  }
  DisposableEffect(hold) { onDispose { hold.release() } }
}
