package com.noma4i.medialayer.ui

internal class ViewOwners {
  private val owners = HashMap<String, ULong>()

  @Synchronized
  fun claim(key: String, token: ULong) {
    owners[key] = token
  }

  @Synchronized
  fun release(key: String, token: ULong): Boolean {
    if (owners[key] != token) return false
    owners.remove(key)
    return true
  }
}
