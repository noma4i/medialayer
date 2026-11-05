@file:Suppress("DEPRECATION")

package com.noma4i.medialayer.ui

import android.os.Looper
import androidx.media3.common.C
import androidx.media3.common.Player
import androidx.media3.test.utils.FakeMediaSource
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.first
import kotlin.concurrent.thread
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
internal class ExoPlayerSlotLifecycleTest : ExoPlayerSlotRig() {
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
