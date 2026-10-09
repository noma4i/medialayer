package com.noma4i.medialayer.ui

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
import org.junit.Assert.assertNotEquals
import org.junit.Test

class OffCallerTest {
  @Test
  fun theBackgroundOpenRunsItsWorkOffTheCallerThread() = runBlocking(Dispatchers.Unconfined) {
    val caller = Thread.currentThread()

    val worker = offCaller { Thread.currentThread() }

    assertNotEquals(caller, worker)
  }
}
