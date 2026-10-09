package com.noma4i.medialayer.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class ViewOwnersTest {
  private val owners = ViewOwners()

  @Test
  fun releasingAnOlderViewOfTheSameKeyDoesNotDetachTheNewerView() {
    owners.claim("k", 1uL)
    owners.claim("k", 2uL)

    assertEquals(false, owners.release("k", 1uL))
    assertEquals(true, owners.release("k", 2uL))
  }

  @Test
  fun aReleasedKeyCanBeClaimedAgain() {
    owners.claim("k", 1uL)
    owners.release("k", 1uL)

    owners.claim("k", 3uL)

    assertEquals(true, owners.release("k", 3uL))
  }
}
