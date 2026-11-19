package com.noma4i.medialayer.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

class CaptureMemoryTest {
  @Test
  fun the_gl_input_the_microphone_and_the_held_take_share_one_total_and_the_last_one_that_does_not_fit_is_refused() {
    val memory = CaptureMemory(100)
    val input = memory.lease(40)
    memory.lease(30)
    val held = memory.lease(30)
    assertEquals(0L, memory.available)
    assertThrows(IllegalStateException::class.java) { memory.lease(1) }
    held.close()
    assertEquals(30L, memory.available)
    input.close()
    assertEquals(70L, memory.available)
    assertEquals(30L, memory.leased)
  }

  @Test
  fun a_huge_lease_is_refused_without_overflowing_the_sum() {
    val memory = CaptureMemory(Long.MAX_VALUE)
    memory.lease(10)
    assertThrows(IllegalStateException::class.java) { memory.lease(Long.MAX_VALUE) }
    assertThrows(IllegalArgumentException::class.java) { memory.lease(0) }
    assertEquals(10L, memory.leased)
  }

  @Test
  fun a_stale_lease_returns_bytes_only_to_its_own_session_and_a_second_close_returns_nothing() {
    val old = CaptureMemory(100)
    val stale = old.lease(60)
    val oldDrained = java.util.concurrent.atomic.AtomicInteger()
    old.close { oldDrained.incrementAndGet() }
    old.close { oldDrained.incrementAndGet() }
    assertThrows(IllegalStateException::class.java) { old.lease(1) }
    assertEquals("not drained while the lease lives", 0, oldDrained.get())
    val resized = CaptureMemory(150)
    val newDrained = java.util.concurrent.atomic.AtomicInteger()
    val fresh = resized.lease(150)
    stale.close()
    stale.close()
    assertEquals("drained once, after the last lease came back", 1, oldDrained.get())
    assertEquals("the old lease does not free the new session", 0L, resized.available)
    assertEquals("and it returned to its own memory", 0L, old.leased)
    resized.close { newDrained.incrementAndGet() }
    assertEquals("the new session waits for its own lease", 0, newDrained.get())
    fresh.close()
    fresh.close()
    assertEquals(1, newDrained.get())
    assertEquals(1, oldDrained.get())
    var now = 0
    CaptureMemory(10).close { now++ }
    assertEquals("a memory without leases is drained at once", 1, now)
  }
}
