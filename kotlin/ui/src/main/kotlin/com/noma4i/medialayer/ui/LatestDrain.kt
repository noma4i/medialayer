package com.noma4i.medialayer.ui

import java.util.concurrent.Executor
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.asExecutor

internal val inputExecutor: Executor = Dispatchers.Default.asExecutor()

/** Drains that have a block scheduled or running; tests wait for zero. */
internal val activeDrains = AtomicInteger()
@Volatile internal var onTestChange: (() -> Unit)? = null

/**
 * Runs [apply] off the caller with at most one running block and one pending value per drain; later values merge into the pending one.
 * Each drain schedules on the shared concurrent executor, so a held [apply] of one drain never delays another drain.
 */
internal class LatestDrain<Value : Any>(private val merge: (Value?, Value) -> Value, private val apply: (Value) -> Unit) {
  private val lock = Any()
  private var pending: Value? = null
  private var scheduled = false

  fun submit(value: Value) {
    val schedules = synchronized(lock) {
      pending = merge(pending, value)
      (!scheduled).also { if (it) scheduled = true }
    }
    if (schedules) {
      activeDrains.incrementAndGet()
      inputExecutor.execute(::run)
    }
  }

  /** One apply per block. A value that arrived meanwhile gets its own block, so the executor is released between applies. */
  private fun run() {
    val next = synchronized(lock) { pending.also { pending = null } }
    try {
      next?.let(apply)
    } finally {
      val again = synchronized(lock) { (pending != null).also { if (!it) scheduled = false } }
      if (again) inputExecutor.execute(::run) else activeDrains.decrementAndGet()
      onTestChange?.invoke()
    }
  }
}
