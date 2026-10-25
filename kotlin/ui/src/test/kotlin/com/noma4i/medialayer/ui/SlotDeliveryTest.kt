package com.noma4i.medialayer.ui

import android.content.Context
import android.os.Looper
import androidx.media3.test.utils.FakeMediaSource
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.PlaybackRoute
import com.noma4i.medialayer.SlotCall
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch
import okhttp3.OkHttpClient
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

@RunWith(MediaLayerTestRunner::class)
class SlotDeliveryTest {
  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val events = RecordingEvents()
  private val route = PlaybackRoute("http://127.0.0.1:9/a.mp4", emptyMap(), null, false, true)
  private val factories = mutableListOf<ExoPlayerSlotFactory>()

  private fun factory() = ExoPlayerSlotFactory(context, OkHttpClient(), startMuted = true).onMainLooper().also {
    it.mediaSourceFor = { FakeMediaSource() }
    it.bind(events)
    factories.add(it)
  }

  private fun idle() = shadowOf(Looper.getMainLooper()).idle()

  private fun launching(slot: ExoPlayerSlot, call: SlotCall): Job = CoroutineScope(Dispatchers.Unconfined).launch { slot.apply(call) }

  @After
  fun releaseEverything() { factories.forEach { it.release() }; idle() }

  @Test
  fun inv_api_42_apply_completes_only_after_the_player_looper_ran_the_command() {
    val slot = factory().createSlot(0u)

    val load = launching(slot, SlotCall.Load("k", route, 1uL))

    assertTrue("apply returned before the looper ran", load.isActive)
    assertNull(slot.player)
    idle()
    assertTrue("apply did not return after the looper ran", load.isCompleted)
    assertNotNull(slot.player)
    assertEquals(1, slot.player!!.mediaItemCount)
  }

  @Test
  fun inv_android_05_pending_loads_do_not_execute_obsolete_sources() {
    val sources = mutableListOf<String>()
    val factory = factory().also { it.mediaSourceFor = { next -> sources.add(next.url); FakeMediaSource() } }
    val slot = factory.createSlot(0u)

    val obsolete = launching(slot, SlotCall.Load("old", route.copy(url = "http://127.0.0.1:9/old.mp4"), 1uL))
    obsolete.cancel()
    launching(slot, SlotCall.Load("new", route, 2uL))
    idle()

    assertEquals(listOf(route.url), sources)
    assertEquals(1, slot.player!!.mediaItemCount)
  }

  @Test
  fun inv_api_42_a_command_after_the_player_looper_quit_still_returns() {
    val factory = ExoPlayerSlotFactory(context, OkHttpClient(), startMuted = true).also {
      it.mediaSourceFor = { FakeMediaSource() }
      it.bind(events)
      factories.add(it)
    }
    val slot = factory.createSlot(0u)
    factory.release()

    repeat(50) {
      Thread.sleep(2)
      kotlinx.coroutines.runBlocking { kotlinx.coroutines.withTimeout(3_000) { slot.apply(SlotCall.Play) } }
    }

    assertNull(slot.player)
  }

  @Test
  fun inv_api_42_a_late_command_after_the_release_returns_and_does_nothing() {
    val factory = factory()
    val slot = factory.createSlot(0u)
    factory.release()
    idle()

    val late = launching(slot, SlotCall.Load("k", route, 1uL))
    idle()

    assertTrue("a late apply never returned after the release", late.isCompleted)
    assertNull(slot.player)
  }

  @Test
  fun inv_api_43_the_events_of_a_load_carry_its_generation_and_the_listener_of_an_older_load_is_removed() {
    val slot = factory().createSlot(3u)
    launching(slot, SlotCall.Load("a", route.copy(looping = false), 5uL)); launching(slot, SlotCall.Play); idle()
    androidx.media3.test.utils.robolectric.TestPlayerRunHelper.runUntilPlaybackState(slot.player!!, androidx.media3.common.Player.STATE_READY)
    val first = slot.loadEvents
    launching(slot, SlotCall.Load("a", route.copy(looping = false), 6uL)); launching(slot, SlotCall.Play); idle()
    androidx.media3.test.utils.robolectric.TestPlayerRunHelper.runUntilPlaybackState(slot.player!!, androidx.media3.common.Player.STATE_READY)
    launching(slot, SlotCall.Unload); idle()

    assertEquals(listOf("ready:3:5", "ready:3:6"), events.generations.filter { it.startsWith("ready") })
    assertFalse("the listener of the old load stayed", first === slot.loadEvents)
    assertNull(slot.loadEvents)
  }
}
