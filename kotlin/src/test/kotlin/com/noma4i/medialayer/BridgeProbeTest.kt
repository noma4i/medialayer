package com.noma4i.medialayer

import org.junit.Assert.assertEquals
import org.junit.Test

class BridgeProbeTest {
  @Test
  fun versionComesFromRust() {
    assertEquals("2.1.0", version())
  }
}
