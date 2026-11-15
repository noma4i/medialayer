@file:Suppress("DEPRECATION")

package com.noma4i.medialayer.ui

import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.datasource.DataSpec
import androidx.media3.datasource.HttpDataSource
import androidx.media3.exoplayer.hls.HlsMediaSource
import androidx.media3.exoplayer.source.ProgressiveMediaSource
import androidx.media3.test.utils.FakeClock
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.first
import com.noma4i.medialayer.PlaybackRoute
import okhttp3.OkHttpClient
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(MediaLayerTestRunner::class)
internal class ExoPlayerSlotPlaybackTest : ExoPlayerSlotRig() {
  @Test
  fun loadPrerollPlayPauseAndLoopMapToThePlayer() {
    val slot = factory().createSlot(0u)

    slot.load("k", route); idle()
    val player = slot.player!!
    assertEquals(1, player.mediaItemCount)
    assertFalse(player.playWhenReady)
    assertEquals(Player.STATE_IDLE, player.playbackState)
    assertEquals(Player.REPEAT_MODE_ONE, player.repeatMode)
    assertSame(slot.output, kotlinx.coroutines.runBlocking { factories.single().players.player("k").first() })

    slot.preroll(); idle()
    org.junit.Assert.assertNotEquals(Player.STATE_IDLE, player.playbackState)
    assertFalse(player.playWhenReady)

    slot.play(); idle()
    assertTrue(player.playWhenReady)

    slot.pause(); idle()
    assertFalse(player.playWhenReady)
  }

  @Test
  fun theRouteDecidesLoopingAndAnEndedEventIsSentOnlyWithoutIt() {
    listOf(true to Player.REPEAT_MODE_ONE, false to Player.REPEAT_MODE_OFF).forEach { (looping, mode) ->
      events.calls.clear()
      val slot = factory().createSlot(5u)
      slot.load("k", route.copy(looping = looping)); idle()
      slot.loadEvents!!.onPlaybackStateChanged(Player.STATE_ENDED)

      assertEquals(mode, slot.player!!.repeatMode)
      assertEquals(looping, slot.sample().isLooping)
      assertEquals(if (looping) emptyList() else listOf("ended:5"), events.calls.toList())
    }
  }

  @Test
  fun inv_idle_05_a_slot_sample_follows_the_playhead_between_events_without_a_tick() {
    val clock = FakeClock(0)
    val slot = factory().also { it.clock = clock }.createSlot(0u)
    slot.load("k", route); idle()
    slot.play(); idle()
    playUntilReady(clock, slot)
    assertTrue("the player must be playing for the extrapolation to run, state ${slot.player!!.playbackState}", slot.player!!.isPlaying)

    val start = slot.sample().positionMs.toLong()
    clock.advanceTime(1000)
    assertEquals(start + 1000.0, slot.sample().positionMs.toDouble(), 150.0)

    slot.pause(); idle()
    val paused = slot.sample().positionMs.toLong()
    clock.advanceTime(1000)
    assertEquals(paused, slot.sample().positionMs.toLong())

    slot.setRate(2f); slot.play(); idle()
    playUntilReady(clock, slot)
    val fast = slot.sample().positionMs.toLong()
    clock.advanceTime(500)
    assertEquals(fast + 1000.0, slot.sample().positionMs.toDouble(), 150.0)

    slot.seek(300u); idle()
    assertEquals(300.0, slot.sample().positionMs.toDouble(), 150.0)

    playUntilReady(clock, slot)
    clock.advanceTime(60_000)
    assertEquals(slot.sample().durationMs, slot.sample().positionMs)
  }

  @Test
  fun aSeekCancelledBeforeItsPostRunsIsNeverApplied() {
    val slot = factory().createSlot(0u)
    slot.load("k", route); idle()

    val job = kotlinx.coroutines.CoroutineScope(kotlinx.coroutines.Dispatchers.Unconfined).launch { slot.apply(com.noma4i.medialayer.SlotCall.Seek(500u)) }
    job.cancel()
    idle()

    assertEquals(0L, slot.player!!.currentPosition)
  }

  @Test
  fun inv_android_20_a_clip_that_is_not_looping_reaches_the_end_and_reports_ended_with_the_generation_of_its_load() {
    val clock = FakeClock(0)
    val slot = factory().also { it.clock = clock }.createSlot(3u)
    slot.load("k", route.copy(looping = false), 7uL); idle()
    slot.play(); idle()
    playUntilReady(clock, slot)
    for (step in 0 until 200) {
      if (slot.player!!.playbackState == Player.STATE_ENDED) break
      clock.advanceTime(1000); idle(); Thread.sleep(10)
    }

    assertEquals(Player.REPEAT_MODE_OFF, slot.player!!.repeatMode)
    assertEquals(Player.STATE_ENDED, slot.player!!.playbackState)
    assertEquals(listOf("ended:3:7"), events.generations.filter { it.startsWith("ended") })
  }

  @Test
  fun a_looping_clip_switched_to_one_shot_reports_its_end_once() {
    val slot = factory().createSlot(3u)
    slot.load("k", route.copy(looping = true), 7uL); idle()

    kotlinx.coroutines.CoroutineScope(kotlinx.coroutines.Dispatchers.Unconfined).launch { slot.apply(com.noma4i.medialayer.SlotCall.Looping(false)) }; idle()
    slot.loadEvents!!.onPlaybackStateChanged(Player.STATE_ENDED); idle()

    assertEquals(Player.REPEAT_MODE_OFF, slot.player!!.repeatMode)
    assertEquals(listOf("ended:3:7"), events.generations.filter { it.startsWith("ended") })
  }

  @Test
  fun inv_api_48_the_slot_sample_is_the_duration_when_the_clip_ended() {
    val slot = factory().createSlot(3u)
    slot.load("k", route.copy(looping = false), 7uL); idle()
    val real = slot.player!!
    slot.player = EndedShort()
    try {
      slot.loadEvents!!.onPlaybackStateChanged(Player.STATE_ENDED)
      val sample = slot.sample()

      assertEquals(10_000uL, sample.durationMs)
      assertEquals(10_000uL, sample.positionMs)
    } finally {
      slot.player = real
    }
  }

  @Test
  fun aPausedAndRewoundSlotThatEndedPlaysAgainFromTheStart() {
    val clock = FakeClock(0)
    val slot = factory().also { it.clock = clock }.createSlot(0u)
    slot.load("k", route.copy(looping = false)); idle()
    slot.play(); idle()
    playUntilReady(clock, slot)
    for (step in 0 until 200) {
      if (slot.player!!.playbackState == Player.STATE_ENDED) break
      clock.advanceTime(1000); idle(); Thread.sleep(10)
    }
    assertEquals(Player.STATE_ENDED, slot.player!!.playbackState)

    slot.pause(); slot.seek(0u); idle()

    assertEquals(0L, slot.player!!.currentPosition)
    assertFalse(slot.player!!.isPlaying)
    assertTrue(slot.player!!.playbackState != Player.STATE_ENDED)
    slot.play(); idle()
    playUntilReady(clock, slot)
    assertTrue(slot.player!!.isPlaying)
    assertTrue(slot.sample().positionMs < slot.sample().durationMs!!)
  }

  @Test
  fun theRateIsThePlaybackSpeedAndGoesBackToNormalWithUnload() {
    val slot = factory().createSlot(0u)
    slot.load("k", route); idle()

    slot.setRate(1.5f); idle()
    assertEquals(1.5f, slot.player!!.playbackParameters.speed, 0f)
    slot.unload(); idle()

    assertEquals(1f, slot.player!!.playbackParameters.speed, 0f)
  }

  @Test
  fun attachGivesTheViewHandleThePlayerDetachTakesItAndSeekMovesThePosition() {
    val factory = factory()
    val slot = factory.createSlot(0u)
    val view = factory.players.newView()
    slot.load("k", route); idle()

    slot.attach(view.token); idle()
    assertSame(slot.output, view.player.value)
    assertTrue(slot.sample().viewAttached)

    slot.seek(1500u); idle()
    assertEquals(1500L, slot.player!!.currentPosition)

    slot.detach(); idle()
    assertNull(view.player.value)
    assertFalse(slot.sample().viewAttached)
  }

  @Test
  fun playerListenerEventsReachThePlatformEventsWithTheSlotNumber() {
    val slot = factory().createSlot(7u)
    slot.load("k", route.copy(looping = false)); idle()
    val listener = slot.loadEvents!!
    val httpError = PlaybackException("x", HttpDataSource.InvalidResponseCodeException(404, "Not Found", null, emptyMap(), DataSpec(android.net.Uri.EMPTY), ByteArray(0)), PlaybackException.ERROR_CODE_IO_BAD_HTTP_STATUS)

    listener.onPlaybackStateChanged(Player.STATE_BUFFERING)
    assertEquals(emptyList<String>(), events.calls.toList())
    listener.onPlaybackStateChanged(Player.STATE_READY)
    listener.onRenderedFirstFrame()
    listener.onPlaybackStateChanged(Player.STATE_BUFFERING)
    listener.onPlaybackStateChanged(Player.STATE_ENDED)
    listener.onPlayerError(httpError)
    listener.onPlayerError(PlaybackException("net", null, PlaybackException.ERROR_CODE_IO_NETWORK_CONNECTION_FAILED))

    assertEquals(
      listOf("ready:7", "visualReady:7", "stalled:7:0", "ended:7", "failed:7:404", "failed:7:0"),
      events.calls.toList()
    )
  }

  @Test
  fun aRealPlayerReachesReadyAndReportsItWithTheSlotNumber() {
    val factory = factory()
    val slot = factory.createSlot(3u)
    factory.renderersFactory = fakeRenderersFactory()
    factory.clock = androidx.media3.test.utils.FakeClock(true)

    slot.load("k", route); slot.play(); idle()
    androidx.media3.test.utils.robolectric.TestPlayerRunHelper.runUntilPlaybackState(slot.player!!, Player.STATE_READY)

    assertTrue("ready:3" in events.calls)
  }

  @Test
  fun inv_audio_04_the_slot_sample_at_the_ready_event_already_carries_the_duration() {
    val factory = factory()
    val slot = factory.createSlot(3u)
    factory.renderersFactory = fakeRenderersFactory()
    factory.clock = androidx.media3.test.utils.FakeClock(true)
    var durationAtReady: ULong? = null
    events.onReady = { durationAtReady = slot.sample().durationMs }

    slot.load("k", route); slot.play(); idle()
    androidx.media3.test.utils.robolectric.TestPlayerRunHelper.runUntilPlaybackState(slot.player!!, Player.STATE_READY)

    assertTrue("ready:3" in events.calls)
    assertNotNull("sample() at ready has no duration", durationAtReady)
  }

  @Test
  fun theRouteHlsFlagPicksTheSourceTypeNotTheExtensionAndHeadersReachTheServer() {
    val sources = HttpMediaSources(context, OkHttpClient())

    assertTrue(sources.create(PlaybackRoute("http://127.0.0.1:9/play/abc", emptyMap(), null, true, false)) is HlsMediaSource)
    assertTrue(sources.create(PlaybackRoute("https://o.test/a.m3u8", emptyMap(), null, false, false)) is ProgressiveMediaSource)

    TestServer { it.respond("200 OK", "x", "Connection: close\r\n") }.use { server ->
      val dataSource = sources.dataSourceFactory(mapOf("Authorization" to "token")).createDataSource()
      dataSource.open(DataSpec(android.net.Uri.parse(server.url)))
      dataSource.close()

      assertEquals("token", server.heads.take().headers["authorization"])
    }
  }
}

private class EndedShort : androidx.media3.test.utils.StubExoPlayer() {
  override fun getPlaybackState(): Int = Player.STATE_ENDED
  override fun getCurrentPosition(): Long = 9_000L
  override fun getDuration(): Long = 10_000L
  override fun getPlayWhenReady(): Boolean = false
  override fun getPlaybackSuppressionReason(): Int = Player.PLAYBACK_SUPPRESSION_REASON_NONE
  override fun getPlaybackParameters(): androidx.media3.common.PlaybackParameters = androidx.media3.common.PlaybackParameters.DEFAULT
}
