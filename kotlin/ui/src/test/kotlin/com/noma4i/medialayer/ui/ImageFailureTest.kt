package com.noma4i.medialayer.ui

import com.noma4i.medialayer.MediaState
import kotlinx.coroutines.CancellationException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ImageFailureTest {
  @Test
  fun aCancelledImageLoadIsNotAFailure() {
    assertNull(imageFailure(CancellationException("cancelled")))
  }

  @Test
  fun aRealErrorIsAFailureWithItsMessage() {
    assertEquals(MediaState.Failed("boom"), imageFailure(IllegalStateException("boom")))
  }
}
