package com.noma4i.medialayer.ui

import java.util.Random
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** The memory of a session lives while any asker holds a lease: random take / close session / restart microphone / new session sequences. */
class CaptureMemoryLifetimeTest {
  private class Session(val memory: CaptureMemory) {
    val leases = ArrayList<AutoCloseable>()
    val drained = AtomicInteger()
    var closing = false
  }

  @Test
  fun random_sequences_never_overlease_never_fail_a_closed_asker_and_drain_after_the_last_lease() {
    val seed = System.getenv("ML_SEED")?.trim()?.toLongOrNull() ?: 1L
    val repeat = "ML_SEED=$seed ./gradlew :ui:testDebugUnitTest --tests '*CaptureMemoryLifetimeTest'"
    val total = 1_000L
    for (round in 0 until 300) {
      val random = Random(seed * 1_000_003L + round)
      val sessions = ArrayList<Session>()
      var current: Session? = null
      var failures = 0
      fun ask(bytes: Long) {
        // the asker always takes the memory of the current session; a closed one ends it quietly
        val session = current ?: return
        try {
          session.leases.add(session.memory.lease(bytes))
        } catch (_: CaptureMemoryClosed) {
          assertTrue("$repeat: a closed session was asked after it was replaced", session.closing)
        } catch (_: IllegalStateException) {
          assertTrue("$repeat: a lease larger than the room is refused only when full", session.memory.available < bytes)
        } catch (error: Throwable) { failures += 1 }
      }
      repeat(60) {
        when (random.nextInt(5)) {
          0 -> { val session = Session(CaptureMemory(total)); sessions.add(session); current = session }
          1 -> ask(1L + random.nextInt(300))
          2 -> current?.let { s -> if (s.leases.isNotEmpty()) s.leases.removeAt(random.nextInt(s.leases.size)).close() }
          3 -> current?.let { s ->
            if (!s.closing) { s.closing = true; s.memory.close { s.drained.incrementAndGet() } }
          }
          else -> ask(1L + random.nextInt(50))
        }
        for (s in sessions) {
          assertTrue("$repeat: leased ${s.memory.leased} exceeds $total", s.memory.leased <= total)
          if (s.closing && s.leases.isNotEmpty() && s.memory.leased > 0) assertEquals("$repeat: drained before the last lease", 0, s.drained.get())
        }
      }
      for (s in sessions) {
        if (!s.closing) { s.closing = true; s.memory.close { s.drained.incrementAndGet() } }
        s.leases.forEach { it.close() }
        s.leases.clear()
        assertEquals("$repeat: the memory closes once, after the last lease", 1, s.drained.get())
        assertEquals(0L, s.memory.leased)
      }
      assertEquals("$repeat: unexpected failures", 0, failures)
    }
  }
}
