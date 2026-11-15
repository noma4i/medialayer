package com.noma4i.medialayer.ui

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeoutOrNull
import java.util.concurrent.CopyOnWriteArrayList

internal class SignallingList<T> : CopyOnWriteArrayList<T>() {
  override fun add(element: T): Boolean = super.add(element).also { TestSignal.fire() }
}

internal object TestSignal {
  private val lock = Any()
  private var version = 0L
  private var next = CompletableDeferred<Unit>()
  init { onTestChange = ::fire }

  fun fire() {
    val previous = synchronized(lock) {
      version++
      next.also { next = CompletableDeferred() }
    }
    previous.complete(Unit)
  }

  fun snapshot(): Long = synchronized(lock) { version }

  suspend fun changedSince(seen: Long) {
    val pending = synchronized(lock) { if (version != seen) null else next }
    pending?.await()
  }

  fun await(what: String, timeoutMs: Long, condition: () -> Boolean) {
    val reached = runBlocking {
      withTimeoutOrNull(timeoutMs) {
        while (true) {
          val seen = snapshot()
          if (condition()) return@withTimeoutOrNull true
          changedSince(seen)
        }
      }
    }
    if (reached != true) throw AssertionError("never became true within $timeoutMs ms: $what")
  }
}
