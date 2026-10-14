package com.noma4i.medialayer

import com.sun.jna.CallbackThreadInitializer
import com.sun.jna.Native

object NativeCallbacks {
  private val keepAttached = CallbackThreadInitializer(true, false, "medialayer")

  @Synchronized
  fun install() {
    Native.setCallbackThreadInitializer(uniffiRustFutureContinuationCallbackImpl, keepAttached)
  }
}
