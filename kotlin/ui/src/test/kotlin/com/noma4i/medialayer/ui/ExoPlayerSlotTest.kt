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
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import com.noma4i.medialayer.PlaybackRoute
import kotlin.concurrent.thread
import okhttp3.OkHttpClient
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(MediaLayerTestRunner::class)
class ExoPlayerSlotTest {
  @Test
  fun inv_android_08_a_disposed_slot_does_not_create_a_live_player_and_a_closed_factory_drops_its_events() {
    val factory = factory()
    val slot = factory.createSlot(0u)
    slot.disposing()
    idle()
    slot.disposing()
    slot.load("late", route)
    idle()
    assertNull(slot.player)
    factory.release()
    assertNull(factory.events)
  }

  @Test
  fun inv_pool_21_dispose_releases_media3_ends_the_owned_thread_and_a_second_dispose_is_safe() = kotlinx.coroutines.runBlocking {
    val threads = mutableListOf<android.os.HandlerThread>()
    val factory = factory().also {
      it.looperFor = { id -> android.os.HandlerThread("test-slot-$id").apply { start(); threads.add(this) }.looper }
    }
    val slot = factory.createSlot(0u)
    slot.apply(com.noma4i.medialayer.SlotCall.Load("k", route, 1uL))
    assertNotNull(slot.player)
    assertNotNull(factory.players.player("k").first())

    slot.dispose()
    slot.dispose()

    assertNull(slot.player)
    assertNull(factory.players.player("k").first())
    assertFalse("the thread of the slot outlived its dispose", threads.single().isAlive)
  }

  @Test
  fun inv_pool_21_the_factory_keeps_no_reference_to_a_disposed_slot() {
    val factory = factory()
    val reference = disposedSlot(factory)
    repeat(100) {
      System.gc()
      if (reference.get() != null) Thread.sleep(10)
    }
    assertNull(reference.get())
  }

  @Test
  fun inv_android_04_publishing_a_does_not_schedule_b() = kotlinx.coroutines.runBlocking {
    var resumes = 0
    val dispatcher = object : kotlinx.coroutines.CoroutineDispatcher() {
      override fun dispatch(context: kotlin.coroutines.CoroutineContext, block: Runnable) {
        resumes++
        block.run()
      }
    }
    val factory = factory()
    val job = kotlinx.coroutines.CoroutineScope(dispatcher).launch {
      factory.players.player("b").collect { }
    }
    resumes = 0
    try {
      factory.createSlot(0u).load("a", route)
      idle()
      assertEquals(0, resumes)
    } finally { job.cancel() }
  }

  @Test
  fun inv_android_06_readahead_survives_pending_player_creation() {
    for (beforeLoad in listOf(true, false)) {
      val slot = factory().createSlot(0u)
      if (beforeLoad) slot.setReadahead(12_000uL)
      slot.load("k", route)
      if (!beforeLoad) slot.setReadahead(12_000uL)
      idle()
      val field = ExoPlayerSlot::class.java.getDeclaredField("loadControl").apply { isAccessible = true }
      val control = field.get(slot) as ReadaheadLoadControl
      val player = androidx.media3.exoplayer.analytics.PlayerId.UNSET
      val timeline = androidx.media3.test.utils.FakeTimeline()
      val period = androidx.media3.exoplayer.source.MediaSource.MediaPeriodId(timeline.getUidOfPeriod(0))
      val parameters = androidx.media3.exoplayer.LoadControl.Parameters(
        player, timeline, period, 0, 12_000_000, 1f, true, false, C.TIME_UNSET, C.TIME_UNSET
      )
      control.onPrepared(player)
      try { assertFalse(control.shouldContinueLoading(parameters)) } finally { control.onReleased(player) }
    }
  }

  @Test
  fun inv_pool_06_default_renderers_allow_the_next_decoder_after_initialization_failure() {
    val renderers = factory().renderersFactory.createRenderers(
      android.os.Handler(Looper.getMainLooper()),
      object : androidx.media3.exoplayer.video.VideoRendererEventListener {},
      object : androidx.media3.exoplayer.audio.AudioRendererEventListener {},
      androidx.media3.exoplayer.text.TextOutput {},
      androidx.media3.exoplayer.metadata.MetadataOutput {}
    ).filterIsInstance<androidx.media3.exoplayer.mediacodec.MediaCodecRenderer>()
    assertTrue(renderers.isNotEmpty())
    val field = androidx.media3.exoplayer.mediacodec.MediaCodecRenderer::class.java.getDeclaredField("enableDecoderFallback").apply { isAccessible = true }
    assertTrue(renderers.all { field.getBoolean(it) })
  }

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
    ExoPlayerSlotFactory(context, OkHttpClient(), startMuted = startMuted, audio = session(focus)).onMainLooper().also {
      it.mediaSourceFor = { FakeMediaSource() }
      it.bind(events)
      factories.add(it)
    }

  private fun idle() = shadowOf(Looper.getMainLooper()).idle()

  private fun disposedSlot(factory: ExoPlayerSlotFactory): java.lang.ref.WeakReference<ExoPlayerSlot> {
    val slot = factory.createSlot(0u)
    slot.load("k", route); idle(); slot.disposing(); idle()
    return java.lang.ref.WeakReference(slot)
  }

  @After
  fun releaseEverything() { factories.forEach { it.release() }; idle() }

  @Test
  fun inv_api_33_sample_does_not_wait_for_create() {
    var sourceBuiltOnMain: Boolean? = null
    val factory = factory().also {
      it.mediaSourceFor = {
        sourceBuiltOnMain = Looper.myLooper() == Looper.getMainLooper()
        FakeMediaSource()
      }
    }
    val worker = java.util.concurrent.Executors.newSingleThreadExecutor()
    try {
      val slot = worker.submit<ExoPlayerSlot> {
        factory.createSlot(0u).also {
          it.load("k", route)
          it.play()
          assertNull(it.player)
          assertNull(sourceBuiltOnMain)
          assertEquals(0uL, it.sample().positionMs)
        }
      }.get(3, java.util.concurrent.TimeUnit.SECONDS)
      idle()
      assertEquals(true, sourceBuiltOnMain)
      assertTrue(slot.player!!.playWhenReady)
      worker.submit {
        assertEquals(0uL, slot.sample().positionMs)
        assertTrue(slot.sample().isLooping)
      }.get(3, java.util.concurrent.TimeUnit.SECONDS)
    } finally {
      worker.shutdownNow()
    }
  }

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

  private fun playUntilReady(clock: FakeClock, slot: ExoPlayerSlot) {
    repeat(100) {
      if (slot.player!!.isPlaying) return
      clock.advanceTime(50); idle(); Thread.sleep(20)
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
  fun muteAndUnmuteChangeOnlyTheVolumeAndNeverTouchAudioFocus() {
    val slot = factory(startMuted = false, focus = MediaAudioFocus.Exclusive).createSlot(0u)
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
    val slot = factory(startMuted = false, focus = MediaAudioFocus.Mix).createSlot(0u)
    val audio = shadowOf(context.getSystemService(Context.AUDIO_SERVICE) as AudioManager)

    slot.load("k", route); slot.play(); settleAudioFocus(); advanceFocus(1000)

    assertTrue(slot.player!!.playWhenReady)
    assertNull(audio.lastAudioFocusRequest)
    assertEquals(emptyList<String>(), focusPort.calls.toList())
  }

  @Test
  fun duckAsksTheLibraryForTheFocusOnceAndThePlayerNever() {
    val slot = factory(startMuted = false, focus = MediaAudioFocus.Duck).createSlot(0u)
    val audio = shadowOf(context.getSystemService(Context.AUDIO_SERVICE) as AudioManager)

    slot.load("k", route); idle(); advanceFocus(1000)

    assertEquals(emptyList<String>(), focusPort.calls.toList())

    slot.play(); settleAudioFocus()

    assertEquals(listOf("request:Duck"), focusPort.calls.toList())
    assertNull(audio.lastAudioFocusRequest)
  }

  @Test
  fun duckFollowsMutePauseAndUnloadOfTheSlot() {
    val slot = factory(startMuted = false, focus = MediaAudioFocus.Duck).createSlot(0u)
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
    val slot = factory(startMuted = true, focus = MediaAudioFocus.Duck).createSlot(0u)

    slot.load("k", route); slot.play(); idle(); advanceFocus(1000)

    assertEquals(emptyList<String>(), focusPort.calls.toList())
  }

  @Test
  fun aSlotThatIsReleasedGivesItsFocusBack() {
    val factory = factory(startMuted = false, focus = MediaAudioFocus.Duck)
    val slot = factory.createSlot(0u)
    slot.load("k", route); slot.play(); idle()

    slot.disposing(); idle(); advanceFocus(300)

    assertEquals(listOf("request:Duck", "abandon"), focusPort.calls.toList())
  }

  @Test
  fun aDebugSlotStartsMutedAndAReleaseSlotStartsAtFullVolume() {
    listOf(true to 0f, false to 1f).forEach { (startMuted, volume) ->
      val slot = factory(startMuted = startMuted).createSlot(0u)
      slot.load("k", route); idle()

      assertEquals(volume, slot.player!!.volume, 0f)
    }
  }

  @Test
  fun unloadDropsTheMediaAndFreesTheKey() {
    val factory = factory()
    val slot = factory.createSlot(0u)
    slot.load("k", route); slot.play(); idle()
    val player = slot.player!!

    assertEquals(1, player.mediaItemCount)
    assertSame(slot.output, kotlinx.coroutines.runBlocking { factory.players.player("k").first() })

    slot.unload(); idle()
    assertEquals(0, player.mediaItemCount)
    assertEquals(Player.STATE_IDLE, player.playbackState)
    assertNull(kotlinx.coroutines.runBlocking { factory.players.player("k").first() })
  }

  @Test
  fun aPoolOfThreeSlotsNeverCreatesMoreThanThreePlayersWhateverTheNumberOfKeys() {
    val factory = factory()
    val slots = (0u..2u).map { factory.createSlot(it) }

    val created = mutableSetOf<Player>()
    repeat(12) { page ->
      val slot = slots[page % 3]
      slot.unload(); slot.load("k$page", route); slot.preroll(); idle()
      created.add(slot.player!!)
    }

    assertEquals(3, created.size)
    for (index in 0..11) {
      val published = kotlinx.coroutines.runBlocking { factory.players.player("k$index").first() }
      assertEquals(index >= 9, published != null)
    }
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
  fun releaseDropsThePlayerThePublishedKeysAndTheListener() {
    val factory = factory()
    val slot = factory.createSlot(0u)
    slot.load("k", route); idle()
    val player = slot.player!!
    val loadEvents = slot.loadEvents!!

    slot.disposing(); idle()

    assertTrue(player.isReleased)
    assertNull(kotlinx.coroutines.runBlocking { factory.players.player("k").first() })
    events.calls.clear()
    loadEvents.onRenderedFirstFrame()
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

internal fun fakeRenderersFactory() = androidx.media3.exoplayer.RenderersFactory { handler, videoListener, _, _, _ ->
  arrayOf<androidx.media3.exoplayer.Renderer>(
    androidx.media3.test.utils.FakeVideoRenderer(androidx.media3.common.util.Clock.DEFAULT.createHandler(handler.looper, null), videoListener)
  )
}
