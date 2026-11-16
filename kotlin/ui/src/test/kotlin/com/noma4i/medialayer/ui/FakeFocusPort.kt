package com.noma4i.medialayer.ui

internal class FakeFocusPort(var grant: Boolean = true, var throwOnRecording: Boolean = false) : AudioFocusPort {
  val calls = SignallingList<String>()
  override var losses = 0
    private set
  private var action: () -> Unit = {}

  override fun onChange(action: () -> Unit) { this.action = action }

  fun gain() { action() }

  fun lose() {
    losses += 1
    calls.add("lost")
    action()
  }

  override fun request(kind: FocusKind): Boolean {
    calls.add("request:$kind")
    if (throwOnRecording && kind == FocusKind.Recording) throw IllegalStateException("focus service died")
    return grant
  }

  override fun abandon() {
    calls.add("abandon")
  }
}
