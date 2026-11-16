package com.noma4i.medialayer.ui

import android.os.Handler
import android.os.Looper

/** Open(generation) or Closed. A delivery runs on the main thread only while its token is the current open generation. */
internal class DeliveryGate(private val main: Handler = Handler(Looper.getMainLooper())) {
  private val lock = Any()
  private var generation = 0L
  private var isOpen = false

  fun open(): Long = synchronized(lock) {
    generation++
    isOpen = true
    generation
  }

  fun close() = synchronized(lock) {
    isOpen = false
    generation++
  }

  fun deliver(token: Long, body: () -> Unit) {
    main.post { if (synchronized(lock) { isOpen && generation == token }) body() }
  }
}
