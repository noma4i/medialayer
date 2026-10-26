package com.noma4i.medialayer.ui

import android.util.Log
import com.noma4i.medialayer.TraceSink
import com.noma4i.medialayer.setTraceSink

internal object MediaTrace {
  const val TAG = "MediaLayerTrace"

  @Volatile var isOn = false
    private set

  internal object Logcat : TraceSink {
    override fun line(text: String) {
      Log.d(TAG, text)
    }
  }

  /** `adb shell setprop log.tag.MediaLayerTrace VERBOSE`, then start the app: the core and the slots write `ML <ms> <source> <text>` lines to logcat. */
  fun install() {
    isOn = Log.isLoggable(TAG, Log.VERBOSE)
    if (isOn) setTraceSink(Logcat)
  }

  inline fun emit(source: String, text: () -> String) {
    if (isOn) Log.d(TAG, "ML ${System.currentTimeMillis()} $source ${text()}")
  }

  internal fun reset() {
    isOn = false
    setTraceSink(null)
  }
}
