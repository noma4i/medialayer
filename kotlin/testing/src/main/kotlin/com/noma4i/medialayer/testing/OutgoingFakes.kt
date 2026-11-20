package com.noma4i.medialayer.testing

import com.noma4i.medialayer.RemoteResult
import com.noma4i.medialayer.ui.OutgoingClock
import com.noma4i.medialayer.ui.OutgoingRemote
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CompletableDeferred

/**
 * A remote for outgoing media tests. Each url answers `NotReady` until [answer] gives its result; a fetch of an unanswered url waits,
 * so a test decides when the remote lands. Install with `Outgoing.install(remote, clock)`.
 */
class FakeOutgoingRemote : OutgoingRemote {
  private val answers = ConcurrentHashMap<String, CompletableDeferred<RemoteResult>>()
  private val fetches = ConcurrentHashMap<String, AtomicInteger>()
  private val running = AtomicInteger()
  private val peak = AtomicInteger()

  /** The most fetches that ran at the same time. */
  val maxConcurrent: Int get() = peak.get()
  val concurrent: Int get() = running.get()
  fun fetchCount(url: String): Int = fetches[url]?.get() ?: 0

  private fun slot(url: String) = answers.getOrPut(url) { CompletableDeferred() }

  /** The next (and every later) fetch of [url] ends with [result]. */
  fun answer(url: String, result: RemoteResult) { slot(url).complete(result) }

  /** Forget the answer of [url]: the next fetch waits again. */
  fun clear(url: String) { answers.remove(url) }

  override suspend fun fetch(url: String, progress: (Double) -> Unit): RemoteResult {
    fetches.getOrPut(url) { AtomicInteger() }.incrementAndGet()
    peak.accumulateAndGet(running.incrementAndGet()) { a, b -> maxOf(a, b) }
    try { return slot(url).await() } finally { running.decrementAndGet() }
  }
}

/** A clock that moves only when the test says so: [advance] releases the delays that came due. */
class ManualOutgoingClock : OutgoingClock {
  private data class Waiter(val due: Long, val gate: CompletableDeferred<Unit>)
  private val lock = Any()
  private var now = 0L
  private val waiters = ArrayList<Waiter>()

  val pending: Int get() = synchronized(lock) { waiters.size }

  override suspend fun delay(millis: Long) {
    val gate = CompletableDeferred<Unit>()
    synchronized(lock) { waiters.add(Waiter(now + millis, gate)) }
    try { gate.await() } finally { synchronized(lock) { waiters.removeAll { it.gate === gate } } }
  }

  fun advance(millis: Long) {
    val due = synchronized(lock) { now += millis; waiters.filter { it.due <= now }.also { waiters.removeAll(it) } }
    due.forEach { it.gate.complete(Unit) }
  }
}
