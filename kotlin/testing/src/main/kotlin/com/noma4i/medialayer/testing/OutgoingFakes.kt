package com.noma4i.medialayer.testing

import com.noma4i.medialayer.RemoteResult
import com.noma4i.medialayer.ui.OutgoingClock
import com.noma4i.medialayer.ui.OutgoingRemote
import kotlinx.coroutines.CompletableDeferred

/**
 * A remote for outgoing items: each url answers at once with what [answer] set, or waits until [complete] when it is held.
 * It counts the fetches in flight so a test can check the two-slot limit.
 */
class FakeOutgoingRemote : OutgoingRemote {
  private val lock = Any()
  private val answers = HashMap<String, RemoteResult>()
  private val held = HashSet<String>()
  private val waiting = HashMap<String, MutableList<CompletableDeferred<RemoteResult>>>()
  private val asked = ArrayList<String>()
  private var running = 0
  private var peak = 0

  val requests: List<String> get() = synchronized(lock) { asked.toList() }
  fun requestCount(url: String): Int = synchronized(lock) { asked.count { it == url } }
  val inFlight: Int get() = synchronized(lock) { running }
  val peakInFlight: Int get() = synchronized(lock) { peak }

  fun answer(url: String, result: RemoteResult) { synchronized(lock) { answers[url] = result } }

  /** The next fetches of [url] wait until [complete]. */
  fun hold(url: String) { synchronized(lock) { held.add(url) } }

  fun complete(url: String, result: RemoteResult = RemoteResult.Complete) {
    val gates = synchronized(lock) {
      held.remove(url)
      answers[url] = result
      waiting.remove(url).orEmpty()
    }
    gates.forEach { it.complete(result) }
  }

  override suspend fun fetch(url: String, progress: (Double) -> Unit): RemoteResult {
    val (gate, immediate) = synchronized(lock) {
      asked.add(url)
      running += 1
      peak = maxOf(peak, running)
      if (url in held) CompletableDeferred<RemoteResult>().also { waiting.getOrPut(url) { ArrayList() }.add(it) } to null
      else null to (answers[url] ?: RemoteResult.Failed(404u, "nothing served at $url"))
    }
    try {
      return gate?.await() ?: immediate!!
    } finally {
      synchronized(lock) { running -= 1 }
    }
  }
}

/** The 5 s re-ask timer under a test's hand: a sleep ends only on [advance]. */
class ManualOutgoingClock : OutgoingClock {
  private val lock = Any()
  private val sleepers = ArrayList<CompletableDeferred<Unit>>()
  private val asked = ArrayList<Long>()

  val sleeping: Int get() = synchronized(lock) { sleepers.size }
  val requestedMilliseconds: List<Long> get() = synchronized(lock) { asked.toList() }

  /** Ends every sleep that is waiting now. */
  fun advance() {
    val ended = synchronized(lock) { sleepers.toList().also { sleepers.clear() } }
    ended.forEach { it.complete(Unit) }
  }

  override suspend fun sleep(millis: Long) {
    val gate = CompletableDeferred<Unit>()
    synchronized(lock) { asked.add(millis); sleepers.add(gate) }
    try { gate.await() } finally { synchronized(lock) { sleepers.remove(gate) } }
  }
}
