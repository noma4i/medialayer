package com.noma4i.medialayer.ui

import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaLayer
import java.util.Collections
import java.util.WeakHashMap

private val claimsByLayer: MutableMap<MediaLayer, KeyClaims> = Collections.synchronizedMap(WeakHashMap())

internal val MediaLayer.claims: KeyClaims get() = synchronized(claimsByLayer) { claimsByLayer.getOrPut(this) { KeyClaims(this) } }

internal class KeyClaims(private val media: MediaLayer) {
  private class Claim(var visible: Boolean = false, var playing: Boolean = false, var muted: Boolean? = null)

  private class Applied(var visible: Boolean = false, var playing: Boolean = false, var muted: Boolean? = null)

  private class Change(val visible: Boolean?, val playing: Boolean?, val muted: Boolean?, val source: MediaItem?)

  private val claims = HashMap<String, HashMap<Any, Claim>>()
  private val applied = HashMap<String, Applied>()
  private val sources = HashMap<String, MediaItem>()

  fun visible(owner: Any, source: MediaItem, visible: Boolean) {
    val change = synchronized(this) {
      if (visible) sources[source.url] = source
      update(owner, source.url) { it.visible = visible }
    }
    forward(change, source.url)
  }

  fun setPlaying(owner: Any, key: String, playing: Boolean) {
    forward(synchronized(this) { update(owner, key) { it.playing = playing } }, key)
  }

  fun setMuted(owner: Any, key: String, muted: Boolean) {
    forward(synchronized(this) { update(owner, key) { it.muted = muted } }, key)
  }

  fun release(owner: Any, key: String) {
    val change = synchronized(this) {
      claims[key]?.remove(owner)
      if (claims[key]?.isEmpty() == true) claims.remove(key)
      diff(key)
    }
    forward(change, key)
  }

  private fun update(owner: Any, key: String, edit: (Claim) -> Unit): Change {
    val claim = claims.getOrPut(key) { HashMap() }.getOrPut(owner) { Claim() }
    edit(claim)
    return diff(key)
  }

  private fun diff(key: String): Change {
    val all = claims[key]?.values.orEmpty()
    val last = applied[key] ?: Applied()
    val visible = all.any { it.visible }.takeIf { it != last.visible }?.also { last.visible = it }
    val playing = all.any { it.playing }.takeIf { it != last.playing }?.also { last.playing = it }
    val opinions = all.mapNotNull { it.muted }
    val muted = if (opinions.isEmpty()) null else opinions.all { it }.takeIf { it != last.muted }?.also { last.muted = it }
    if (all.isEmpty()) applied.remove(key) else applied[key] = last
    val source = if (all.isEmpty()) sources.remove(key) else sources[key]
    return Change(visible, playing, muted, source)
  }

  private fun forward(change: Change, key: String) {
    if (change.visible == true) change.source?.let { media.visible(it, true) }
    change.playing?.let { media.setPlaying(key, it) }
    change.muted?.let { media.setMuted(key, it) }
    if (change.visible == false) change.source?.let { media.visible(it, false) }
  }
}
