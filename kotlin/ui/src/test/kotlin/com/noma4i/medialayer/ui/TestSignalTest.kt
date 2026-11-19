package com.noma4i.medialayer.ui

import org.junit.Test
import java.util.concurrent.atomic.AtomicBoolean

class TestSignalTest {
  @Test
  fun a_wait_sees_a_state_that_changes_without_a_fire() {
    val flag = AtomicBoolean(false)
    Thread { Thread.sleep(100); flag.set(true) }.start()

    TestSignal.await("flag set without a fire", 2_000) { flag.get() }
  }
}
