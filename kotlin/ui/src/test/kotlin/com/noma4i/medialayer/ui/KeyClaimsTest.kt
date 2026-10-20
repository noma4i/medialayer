package com.noma4i.medialayer.ui

import com.noma4i.medialayer.CacheStrategy
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaItem
import org.junit.Assert.assertEquals
import org.junit.Test
import java.lang.ref.WeakReference
import org.junit.Assert.assertNull

class KeyClaimsTest {
  @Test
  fun inv_native_01_claims_do_not_retain_their_layer() {
    val (reference, retainedClaims) = run {
      val media = RecordingLayer()
      WeakReference(media) to media.claims
    }
    repeat(20) {
      System.gc()
      if (reference.get() == null) return@repeat
      Thread.sleep(10)
    }
    assertNull(reference.get())
    retainedClaims.release(Any(), "unused")
  }

  private val layer = RecordingLayer()
  private val claims = KeyClaims(layer)
  private val item = MediaItem(kind = Kind.VIDEO, url = "https://o.test/a.mp4", headers = emptyMap(), cover = null, blurHash = null, strategy = CacheStrategy.WINDOW)

  @Test
  fun twoViewsOfOneKeyKeepItVisibleUntilTheLastOneLeaves() {
    val first = Any()
    val second = Any()

    claims.visible(first, item, true)
    claims.visible(second, item, true)
    claims.visible(first, item, false)
    assertEquals(listOf("visible:${item.url}:true"), layer.calls.toList())

    claims.visible(second, item, false)
    assertEquals(listOf("visible:${item.url}:true", "visible:${item.url}:false"), layer.calls.toList())
  }

  @Test
  fun aPeekedKeyIsForwardedOnceAndWithdrawnWhenItsLastViewLeaves() {
    val first = Any()
    val second = Any()

    claims.setPeeking(first, "k", true)
    claims.setPeeking(second, "k", true)
    claims.release(first, "k")
    assertEquals(listOf("peeking:k:true"), layer.calls.toList())

    claims.release(second, "k")
    assertEquals(listOf("peeking:k:true", "peeking:k:false"), layer.calls.toList())
  }

  @Test
  fun aViewThatDoesNotPlayNeverStopsAnotherViewOfTheSameKey() {
    val playing = Any()
    val idle = Any()

    claims.setPlaying(playing, "k", true)
    claims.setPlaying(idle, "k", false)
    assertEquals(listOf("playing:k:true"), layer.calls.toList())

    claims.setPlaying(playing, "k", false)
    assertEquals(listOf("playing:k:true", "playing:k:false"), layer.calls.toList())
  }

  @Test
  fun aKeyIsMutedOnlyWhileEveryViewOfItAsksForMute() {
    val first = Any()
    val second = Any()

    claims.setMuted(first, "k", true)
    claims.setMuted(second, "k", false)
    claims.setMuted(second, "k", true)

    assertEquals(listOf("muted:k:true", "muted:k:false", "muted:k:true"), layer.calls.toList())
  }

  @Test
  fun aViewThatLeavesDropsItsPlayClaim() {
    val owner = Any()
    claims.setPlaying(owner, "k", true)

    claims.release(owner, "k")

    assertEquals(listOf("playing:k:true", "playing:k:false"), layer.calls.toList())
  }

  @Test
  fun theLastViewThatLeavesStopsThePlayBeforeItWithdrawsTheKey() {
    val owner = Any()
    claims.visible(owner, item, true)
    claims.setPlaying(owner, item.url, true)

    claims.release(owner, item.url)

    assertEquals(
      listOf("visible:${item.url}:true", "playing:${item.url}:true", "playing:${item.url}:false", "visible:${item.url}:false"),
      layer.calls.toList()
    )
  }
}
