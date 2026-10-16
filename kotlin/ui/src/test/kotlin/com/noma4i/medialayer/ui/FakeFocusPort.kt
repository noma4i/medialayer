package com.noma4i.medialayer.ui

import java.util.concurrent.CopyOnWriteArrayList

internal class FakeFocusPort(var grant: Boolean = true, var throwOnRecording: Boolean = false) : AudioFocusPort {
  val calls = CopyOnWriteArrayList<String>()

  override fun request(kind: FocusKind): Boolean {
    calls.add("request:$kind")
    if (throwOnRecording && kind == FocusKind.Recording) throw IllegalStateException("focus service died")
    return grant
  }

  override fun abandon() {
    calls.add("abandon")
  }
}
