package com.noma4i.medialayer

import org.junit.Assert.assertEquals
import org.junit.Test

class BridgeProbeTest {
  @Test
  fun versionComesFromRust() {
    assertEquals("3.7.0", version())
  }
}
