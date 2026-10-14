package com.noma4i.medialayer.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class VideoProgressTest {
  private val table: List<Triple<ULong, ULong?, Double?>> = listOf(
    Triple(0uL, 8000uL, 0.0),
    Triple(2000uL, 8000uL, 0.25),
    Triple(4000uL, 8000uL, 0.5),
    Triple(8000uL, 8000uL, 1.0),
    Triple(9000uL, 8000uL, 1.0),
    Triple(3000uL, null, null),
    Triple(3000uL, 0uL, null)
  )

  @Test
  fun positionOverDurationIsTheFractionAndAMissingDurationHasNone() {
    for ((position, duration, fraction) in table) {
      val actual = VideoProgress.fraction(position, duration)
      if (fraction == null) assertNull("$position/$duration", actual) else assertEquals("$position/$duration", fraction, actual!!, 0.0)
    }
  }

  @Test
  fun theFractionIsReportedTenTimesASecond() {
    assertEquals(100L, VideoProgress.INTERVAL_MS)
  }
}
