package com.noma4i.medialayer.ui

internal interface AudioFocusPort {
  fun onChange(action: () -> Unit)
  fun request(kind: FocusKind): Boolean
  fun setCommunication(on: Boolean, speaker: Boolean)

  fun abandon()

  val losses: Int
}
