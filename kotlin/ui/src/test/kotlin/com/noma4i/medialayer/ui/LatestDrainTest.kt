package com.noma4i.medialayer.ui

import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class LatestDrainTest {
  @Test
  fun heldApplyKeepsOneRunningAndOneMergedPendingValue() {
    val applied = CopyOnWriteArrayList<Int>()
    val entered = CountDownLatch(1)
    val release = CountDownLatch(1)
    val appliedLast = CountDownLatch(1)
    val drain = LatestDrain<Int>({ _, next -> next }) { value ->
      applied.add(value)
      if (value == 10_000) appliedLast.countDown()
      if (value == 0) {
        entered.countDown()
        release.await(5, TimeUnit.SECONDS)
      }
    }

    drain.submit(0)
    assertTrue(entered.await(5, TimeUnit.SECONDS))
    for (value in 1..10_000) drain.submit(value)
    release.countDown()

    assertTrue("last value applied", appliedLast.await(5, TimeUnit.SECONDS))
    assertEquals(listOf(0, 10_000), applied.toList())
  }

  @Test
  fun applyRunsOffTheCallerThread() {
    val caller = Thread.currentThread()
    val ran = CountDownLatch(1)
    var thread: Thread? = null
    val drain = LatestDrain<Int>({ _, next -> next }) { thread = Thread.currentThread(); ran.countDown() }

    drain.submit(1)

    assertTrue(ran.await(5, TimeUnit.SECONDS))
    assertTrue(thread !== caller)
  }
}
