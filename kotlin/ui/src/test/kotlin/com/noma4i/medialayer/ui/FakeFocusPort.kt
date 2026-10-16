package com.noma4i.medialayer.ui

import java.util.concurrent.CopyOnWriteArrayList

internal class FakeFocusPort(var grant: Boolean = true) : AudioFocusPort {
  val calls = CopyOnWriteArrayList<String>()

  override fun request(kind: FocusKind): Boolean {
    calls.add("request:$kind")
    return grant
  }

  override fun abandon() {
    calls.add("abandon")
  }
}
