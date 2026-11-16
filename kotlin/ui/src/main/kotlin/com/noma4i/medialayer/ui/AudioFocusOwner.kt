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
  private val scope: CoroutineScope,
  private val communication: () -> Pair<Boolean, Boolean>,
  private val callLive: (Boolean) -> Unit
) {
  private val lock = Any()
  private var held: FocusKind? = null
  private var pending: Job? = null
  private var seenLosses = 0
  private var appliedCommunication = false to false
  private var managed = false

  fun reconcile(): Boolean = synchronized(lock) {
    if (managed) return true
    val target = want()
    val losses = port.losses
    if (losses != seenLosses) {
      seenLosses = losses
      held = null
      callLive(false)
      if (communication().first) appliedCommunication = false to false
    }
    val desiredCommunication = communication()
    if (desiredCommunication != appliedCommunication) {
      port.setCommunication(desiredCommunication.first, desiredCommunication.second)
      appliedCommunication = desiredCommunication
    }
    if (target == null) {
      if (held != null && pending == null) scheduleRelease()
      return true
    }
    if (held != target) {
      val granted = runCatching { port.request(target) }.getOrDefault(false)
      MediaTrace.emit("audio") { "focus $target granted=$granted" }
      if (!granted) { callLive(false); return false }
    }

    pending?.cancel()
    pending = null
    held = target
    callLive(target == FocusKind.Call)
    true
  }

  fun setManaged(value: Boolean): Unit = synchronized(lock) {
    if (managed == value) return
    if (value) {
      pending?.cancel()
      pending = null
      if (held != null) port.abandon()
      held = null
      if (appliedCommunication.first) port.setCommunication(false, false)
      appliedCommunication = false to false
    }
    managed = value
  }

  fun close() = synchronized(lock) {
    pending?.cancel()
    pending = null
    if (held != null) {
      port.abandon()
      MediaTrace.emit("audio") { "focus abandoned" }
    }
    held = null
    callLive(false)
    if (appliedCommunication.first) {
      port.setCommunication(false, false)
      appliedCommunication = false to false
    }
  }

  private fun scheduleRelease() {
    val job = scope.launch(start = CoroutineStart.LAZY) {
      delay(releaseDelayMs)
      release(coroutineContext[Job]!!)
    }
    pending = job
    job.start()
  }

  private fun release(self: Job) {
    synchronized(lock) {
      if (pending !== self) return
      pending = null
      if (want() != null || held == null) return

      port.abandon()
      MediaTrace.emit("audio") { "focus abandoned" }
      held = null
    }
  }
}
