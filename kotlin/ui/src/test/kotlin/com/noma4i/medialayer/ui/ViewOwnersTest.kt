package com.noma4i.medialayer.ui

import org.junit.Assert.assertEquals
import org.junit.Test

class ViewOwnersTest {
  private val owners = ViewOwners()

  @Test
  fun releasingAnOlderViewOfTheSameKeyDoesNotDetachTheNewerView() {
    owners.claim("k", 1uL, false)
    owners.claim("k", 2uL, true)

    assertEquals(ViewRelease.None, owners.release("k", 1uL))
    assertEquals(ViewRelease.Detach, owners.release("k", 2uL))
  }

  @Test
  fun aReleasedKeyCanBeClaimedAgain() {
    owners.claim("k", 1uL, false)
    owners.release("k", 1uL)

    owners.claim("k", 3uL, false)

    assertEquals(ViewRelease.Detach, owners.release("k", 3uL))
  }

  @Test
  fun aNewIdleHostDoesNotTakeTheViewAndAPlayingHostHandsItBack() {
    assertEquals(true, owners.claim("k", 1uL, false))
    assertEquals(false, owners.claim("k", 2uL, false))
    assertEquals(2uL, owners.prefer("k", 2uL))
    assertEquals(null, owners.prefer("k", 2uL))
    assertEquals(ViewRelease.Handover(1uL), owners.release("k", 2uL))
    assertEquals(ViewRelease.Detach, owners.release("k", 1uL))
    assertEquals(ViewRelease.None, owners.release("k", 1uL))
  }

  @Test
  fun aNewPlayingHostTakesTheViewAndHandsItBackWhenItLeaves() {
    assertEquals(true, owners.claim("k", 1uL, false))
    assertEquals(true, owners.claim("k", 2uL, true))
    assertEquals(ViewRelease.Handover(1uL), owners.release("k", 2uL))
  }
}
