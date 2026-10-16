@file:Suppress("DEPRECATION")

package com.noma4i.medialayer.ui

import android.content.Context
import android.media.AudioManager
import android.os.Looper
import androidx.media3.common.C
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.datasource.DataSpec
import androidx.media3.datasource.HttpDataSource
import androidx.media3.exoplayer.hls.HlsMediaSource
import androidx.media3.exoplayer.source.ProgressiveMediaSource
import androidx.media3.test.utils.FakeClock
import androidx.media3.test.utils.FakeMediaSource
import androidx.test.core.app.ApplicationProvider
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import com.noma4i.medialayer.PlaybackRoute
import kotlin.concurrent.thread
import okhttp3.OkHttpClient
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(RobolectricTestRunner::class)
class ExoPlayerSlotTest {
  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val events = RecordingEvents()
  private val route = PlaybackRoute("http://127.0.0.1:9/a.mp4", emptyMap(), null, false, true)
  private val factories = mutableListOf<ExoPlayerSlotFactory>()

  private val focusPort = FakeFocusPort()
  private val focusScope = TestScope()

  private fun session(focus: MediaAudioFocus) = MediaAudioSession(focus, focusPort, 300, focusScope)

  private fun settleAudioFocus() = repeat(20) { idle(); Thread.sleep(10) }

  private fun advanceFocus(ms: Long) {
    focusScope.advanceTimeBy(ms)
    focusScope.runCurrent()
  }

  private fun factory(startMuted: Boolean = true, focus: MediaAudioFocus = MediaAudioFocus.Mix) =
    ExoPlayerSlotFactory(context, OkHttpClient(), startMuted = startMuted, audio = session(focus)).also {
      it.mediaSourceFor = { FakeMediaSource() }
      it.bind(events)
      factories.add(it)
    }

  private fun idle() = shadowOf(Looper.getMainLooper()).idle()

  @After
  fun releaseEverything() { factories.forEach { it.release() }; idle() }

  @Test
  fun commandsFromAnyThreadRunOnTheMainThreadAfterThePostedWork() {
    var sourceBuiltOnMain: Boolean? = null
    val factory = factory().also { it.mediaSourceFor = { sourceBuiltOnMain = Looper.myLooper() == Looper.getMainLooper(); FakeMediaSource() } }
    val slot = factory.create(0u)

    thread { slot.load("k", route); slot.play() }.join()

    assertNull(slot.player)
    assertNull(sourceBuiltOnMain)
    idle()
    assertEquals(true, sourceBuiltOnMain)
    assertTrue(slot.player!!.playWhenReady)
  }

  @Test
  fun loadPrerollPlayPauseAndLoopMapToThePlayer() {
    val slot = factory().create(0u)

    slot.load("k", route); idle()
    val player = slot.player!!
    assertEquals(1, player.mediaItemCount)
    assertFalse(player.playWhenReady)
    assertEquals(Player.STATE_IDLE, player.playbackState)
    assertEquals(Player.REPEAT_MODE_ONE, player.repeatMode)
    assertSame(player, factories.single().players.keyed.value["k"])

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
      val slot = factory().create(5u)
      slot.load("k", route.copy(looping = looping)); idle()
      slot.listener.onPlaybackStateChanged(Player.STATE_ENDED)

      assertEquals(mode, slot.player!!.repeatMode)
      assertEquals(looping, slot.sample().isLooping)
      assertEquals(if (looping) emptyList() else listOf("ended:5"), events.calls.toList())
    }
  }

  private fun playUntilReady(clock: FakeClock, slot: ExoPlayerSlot) {
    repeat(100) {
      if (slot.player!!.isPlaying) return
      clock.advanceTime(50); idle(); Thread.sleep(20)
    }
  }

  @Test
  fun inv_idle_05_a_slot_sample_follows_the_playhead_between_events_without_a_tick() {
    val clock = FakeClock(0)
    val slot = factory().also { it.clock = clock }.create(0u)
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
  fun theRateIsThePlaybackSpeedAndGoesBackToNormalWithUnload() {
    val slot = factory().create(0u)
    slot.load("k", route); idle()

    slot.setRate(1.5f); idle()
    assertEquals(1.5f, slot.player!!.playbackParameters.speed, 0f)
    slot.unload(); idle()

    assertEquals(1f, slot.player!!.playbackParameters.speed, 0f)
  }

  @Test
  fun attachGivesTheViewHandleThePlayerDetachTakesItAndSeekMovesThePosition() {
    val factory = factory()
    val slot = factory.create(0u)
    val view = factory.players.newView()
    slot.load("k", route); idle()

    slot.attach(view.token); idle()
    assertSame(slot.player, view.player.value)
    assertTrue(slot.sample().viewAttached)

    slot.seek(1500u); idle()
    assertEquals(1500L, slot.player!!.currentPosition)

    slot.detach(); idle()
    assertNull(view.player.value)
    assertFalse(slot.sample().viewAttached)
  }

  @Test
  fun muteAndUnmuteChangeOnlyTheVolumeAndNeverTouchAudioFocus() {
    val slot = factory(startMuted = false, focus = MediaAudioFocus.Exclusive).create(0u)
    val audio = shadowOf(context.getSystemService(Context.AUDIO_SERVICE) as AudioManager)
    slot.load("k", route); slot.play(); idle()
    untilIdle { audio.lastAudioFocusRequest != null }
    val grant = audio.lastAudioFocusRequest
    assertEquals(AudioManager.AUDIOFOCUS_GAIN, grant!!.audioFocusRequest.focusGain)
    assertEquals(emptyList<String>(), focusPort.calls.toList())

    repeat(10) { slot.mute(); idle(); assertEquals(0f, slot.player!!.volume, 0f); slot.unmute(); idle(); assertEquals(1f, slot.player!!.volume, 0f) }
    slot.unload(); idle()
    untilIdle { audio.lastAbandonedAudioFocusRequest != null }

    assertSame(grant, audio.lastAudioFocusRequest)
  }

  @Test
  fun mixPlaysWithSoundAndNeverRequestsFocusFromThePlayerOrTheLibrary() {
    val slot = factory(startMuted = false, focus = MediaAudioFocus.Mix).create(0u)
    val audio = shadowOf(context.getSystemService(Context.AUDIO_SERVICE) as AudioManager)

    slot.load("k", route); slot.play(); settleAudioFocus(); advanceFocus(1000)

    assertTrue(slot.player!!.playWhenReady)
    assertNull(audio.lastAudioFocusRequest)
    assertEquals(emptyList<String>(), focusPort.calls.toList())
  }

  @Test
  fun duckAsksTheLibraryForTheFocusOnceAndThePlayerNever() {
    val slot = factory(startMuted = false, focus = MediaAudioFocus.Duck).create(0u)
    val audio = shadowOf(context.getSystemService(Context.AUDIO_SERVICE) as AudioManager)

    slot.load("k", route); idle(); advanceFocus(1000)

    assertEquals(emptyList<String>(), focusPort.calls.toList())

    slot.play(); settleAudioFocus()

    assertEquals(listOf("request:Duck"), focusPort.calls.toList())
    assertNull(audio.lastAudioFocusRequest)
  }

  @Test
  fun duckFollowsMutePauseAndUnloadOfTheSlot() {
    val slot = factory(startMuted = false, focus = MediaAudioFocus.Duck).create(0u)
    slot.load("k", route); slot.play(); idle()

    slot.mute(); idle(); advanceFocus(300)

    assertEquals(listOf("request:Duck", "abandon"), focusPort.calls.toList())

    slot.unmute(); idle()

    assertEquals(listOf("request:Duck", "abandon", "request:Duck"), focusPort.calls.toList())

    slot.pause(); idle(); advanceFocus(300)

    assertEquals(listOf("request:Duck", "abandon", "request:Duck", "abandon"), focusPort.calls.toList())

    slot.play(); idle(); slot.unload(); idle(); advanceFocus(300)

    assertEquals(listOf("request:Duck", "abandon", "request:Duck", "abandon", "request:Duck", "abandon"), focusPort.calls.toList())
  }

  @Test
  fun aMutedSlotStartedBySilentNeverTakesTheFocus() {
    val slot = factory(startMuted = true, focus = MediaAudioFocus.Duck).create(0u)

    slot.load("k", route); slot.play(); idle(); advanceFocus(1000)

    assertEquals(emptyList<String>(), focusPort.calls.toList())
  }

  @Test
  fun aSlotThatIsReleasedGivesItsFocusBack() {
    val factory = factory(startMuted = false, focus = MediaAudioFocus.Duck)
    val slot = factory.create(0u)
    slot.load("k", route); slot.play(); idle()

    factory.release(); idle(); advanceFocus(300)

    assertEquals(listOf("request:Duck", "abandon"), focusPort.calls.toList())
  }

  @Test
  fun aDebugSlotStartsMutedAndAReleaseSlotStartsAtFullVolume() {
    listOf(true to 0f, false to 1f).forEach { (startMuted, volume) ->
      val slot = factory(startMuted = startMuted).create(0u)
      slot.load("k", route); idle()

      assertEquals(volume, slot.player!!.volume, 0f)
    }
  }

  @Test
  fun unloadDropsTheMediaAndFreesTheKey() {
    val factory = factory()
    val slot = factory.create(0u)
    slot.load("k", route); slot.play(); idle()
    val player = slot.player!!

    assertEquals(1, player.mediaItemCount)
    assertSame(player, factory.players.keyed.value["k"])

    slot.unload(); idle()
    assertEquals(0, player.mediaItemCount)
    assertEquals(Player.STATE_IDLE, player.playbackState)
    assertTrue(factory.players.keyed.value.isEmpty())
  }

  @Test
  fun aPoolOfThreeSlotsNeverCreatesMoreThanThreePlayersWhateverTheNumberOfKeys() {
    val factory = factory()
    val slots = (0u..2u).map { factory.create(it) }

    val created = mutableSetOf<Player>()
    repeat(12) { page ->
      val slot = slots[page % 3]
      slot.unload(); slot.load("k$page", route); slot.preroll(); idle()
      created.add(slot.player!!)
    }

    assertEquals(3, created.size)
    assertEquals(3, factory.players.keyed.value.size)
    assertEquals(setOf("k9", "k10", "k11"), factory.players.keyed.value.keys)
  }

  @Test
  fun playerListenerEventsReachThePlatformEventsWithTheSlotNumber() {
    val slot = factory().create(7u)
    slot.load("k", route.copy(looping = false)); idle()
    val listener = slot.listener
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
    val slot = factory.create(3u)
    factory.renderersFactory = fakeRenderersFactory()
    factory.clock = androidx.media3.test.utils.FakeClock(true)

    slot.load("k", route); slot.play(); idle()
    androidx.media3.test.utils.robolectric.TestPlayerRunHelper.runUntilPlaybackState(slot.player!!, Player.STATE_READY)

    assertTrue("ready:3" in events.calls)
  }

  @Test
  fun releaseDropsThePlayerThePublishedKeysAndTheListener() {
    val factory = factory()
    val slot = factory.create(0u)
    slot.load("k", route); idle()
    val player = slot.player!!

    factory.release(); idle()

    assertTrue(player.isReleased)
    assertTrue(factory.players.keyed.value.isEmpty())
    events.calls.clear()
    slot.listener.onRenderedFirstFrame()
    assertEquals(emptyList<String>(), events.calls.toList())
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

  private fun untilIdle(condition: () -> Boolean) {
    val deadline = System.nanoTime() + 10_000_000_000L
    while (!condition()) {
      check(System.nanoTime() < deadline) { "condition never held" }
      idle()
      Thread.yield()
    }
  }

  @Test
  fun theLoadControlStopsLoadingAtTheReadaheadTheCoreSetsAndWaitsForTheStartBuffers() {
    val control = LoadControlConfig().create(androidx.media3.exoplayer.upstream.DefaultAllocator(true, C.DEFAULT_BUFFER_SEGMENT_SIZE))
    val player = androidx.media3.exoplayer.analytics.PlayerId.UNSET
    val timeline = androidx.media3.test.utils.FakeTimeline()
    val period = androidx.media3.exoplayer.source.MediaSource.MediaPeriodId(timeline.getUidOfPeriod(0))
    fun parameters(bufferedUs: Long, rebuffering: Boolean = false) = androidx.media3.exoplayer.LoadControl.Parameters(
      player, timeline, period, 0, bufferedUs, 1f, true, rebuffering, C.TIME_UNSET, C.TIME_UNSET
    )
    control.onPrepared(player)

    assertTrue(control.shouldContinueLoading(parameters(30_000_000)))
    control.setReadaheadMs(12_000)
    assertTrue(control.shouldContinueLoading(parameters(11_999_000)))
    assertFalse(control.shouldContinueLoading(parameters(12_000_000)))
    assertFalse(control.shouldStartPlayback(parameters(999_000)))
    assertTrue(control.shouldStartPlayback(parameters(1_000_000)))
    assertFalse(control.shouldStartPlayback(parameters(1_999_000, rebuffering = true)))
    assertTrue(control.shouldStartPlayback(parameters(2_000_000, rebuffering = true)))
    assertEquals(0L, control.getBackBufferDurationUs(player))
  }
}

private fun fakeRenderersFactory() = androidx.media3.exoplayer.RenderersFactory { handler, videoListener, _, _, _ ->
  arrayOf<androidx.media3.exoplayer.Renderer>(
    androidx.media3.test.utils.FakeVideoRenderer(androidx.media3.common.util.Clock.DEFAULT.createHandler(handler.looper, null), videoListener)
  )
}
