package com.noma4i.medialayer.ui

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

internal class AudioFocusOwner(
  private val port: AudioFocusPort,
  private val want: () -> FocusKind?,
  private val releaseDelayMs: Long,
  private val scope: CoroutineScope
) {
  private val lock = Any()
  private var held: FocusKind? = null
  private var pending: Job? = null

  fun reconcile(): Boolean = synchronized(lock) {
    val target = want()
    if (target == null) {
      if (held != null && pending == null) scheduleRelease()
      return true
    }
    if (held != target && !runCatching { port.request(target) }.getOrDefault(false)) return false

    pending?.cancel()
    pending = null
    held = target
    true
  }

  fun close() = synchronized(lock) {
    pending?.cancel()
    pending = null
    if (held != null) port.abandon()
    held = null
  }

  private fun scheduleRelease() {
    val job = scope.launch(start = CoroutineStart.LAZY) {
      delay(releaseDelayMs)
      release(coroutineContext[Job]!!)
    }
    pending = job
    job.start()
  }

  private fun release(self: Job) = synchronized(lock) {
    if (pending !== self) return
    pending = null
    if (want() != null || held == null) return

    port.abandon()
    held = null
  }
}
