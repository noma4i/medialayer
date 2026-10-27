package com.noma4i.medialayer.ui

import java.util.concurrent.atomic.AtomicBoolean

private val recordedComposeTimeout = AtomicBoolean()

internal fun reportComposeTimeout(error: Throwable, state: String): Nothing {
  error.addSuppressed(IllegalStateException(state))
  if (recordedComposeTimeout.compareAndSet(false, true)) {
    val threads = Thread.getAllStackTraces().entries
      .filter { (thread, _) -> thread.name.contains("main", ignoreCase = true) || thread.name.contains("DefaultDispatcher") || thread.name.contains("Ffi") }
      .joinToString("\n") { (thread, stack) -> "${thread.name} ${thread.state}\n${stack.take(20).joinToString("\n")}" }
    error.addSuppressed(IllegalStateException(threads))
  }
  throw error
}
