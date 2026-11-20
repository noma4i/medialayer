package com.noma4i.medialayer.ui

import java.util.concurrent.atomic.AtomicBoolean

private val recordedComposeTimeout = AtomicBoolean()

/** Every live thread with its state and up to 40 frames, once per JVM; also printed to System.err. Null after the first call. */
internal fun dumpAllThreadsOnce(): String? {
  if (!recordedComposeTimeout.compareAndSet(false, true)) return null
  val dump = Thread.getAllStackTraces().entries
    .joinToString("\n") { (thread, stack) -> "${thread.name} ${thread.state}\n${stack.take(40).joinToString("\n")}" }
  System.err.println("THREAD DUMP ON TEST TIMEOUT\n$dump")
  return dump
}

internal fun reportComposeTimeout(error: Throwable, state: String): Nothing {
  error.addSuppressed(IllegalStateException(state))
  dumpAllThreadsOnce()?.let { error.addSuppressed(IllegalStateException(it)) }
  throw error
}
