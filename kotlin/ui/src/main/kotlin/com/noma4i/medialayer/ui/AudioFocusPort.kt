package com.noma4i.medialayer.ui

internal interface AudioFocusPort {
  fun request(kind: FocusKind): Boolean

  fun abandon()

  val losses: Int
}
