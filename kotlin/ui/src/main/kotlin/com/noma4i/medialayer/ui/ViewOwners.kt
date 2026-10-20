package com.noma4i.medialayer.ui

internal class ViewOwners {
  private val owners = HashMap<String, MutableList<ULong>>()

  @Synchronized
  fun claim(key: String, token: ULong, playing: Boolean): Boolean {
    val stack = owners.getOrPut(key) { mutableListOf() }
    stack.remove(token)
    val first = stack.isEmpty()
    stack.add(if (playing) stack.size else 0, token)
    return first || playing
  }

  @Synchronized
  fun prefer(key: String, token: ULong): ULong? {
    val stack = owners[key] ?: return null
    if (stack.lastOrNull() == token || !stack.remove(token)) return null
    stack.add(token)
    return token
  }

  @Synchronized
  fun release(key: String, token: ULong): ViewRelease {
    val stack = owners[key] ?: return ViewRelease.None
    val selected = stack.lastOrNull() == token
    stack.remove(token)
    if (stack.isEmpty()) owners.remove(key)
    if (!selected) return ViewRelease.None
    return stack.lastOrNull()?.let { ViewRelease.Handover(it) } ?: ViewRelease.Detach
  }
}

internal sealed interface ViewRelease {
  data object None : ViewRelease
  data object Detach : ViewRelease
  data class Handover(val token: ULong) : ViewRelease
}
