package com.noma4i.medialayer.ui

import android.content.Context
import android.os.Looper
import androidx.media3.test.utils.FakeClock
import androidx.media3.test.utils.FakeMediaSource
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.PlaybackRoute
import okhttp3.OkHttpClient
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

@RunWith(MediaLayerTestRunner::class)
class PlayerSnapshotTest {
  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val route = PlaybackRoute("http://127.0.0.1:9/a.mp4", emptyMap(), null, false, true)
  private val factory = ExoPlayerSlotFactory(context, OkHttpClient(), startMuted = true).onMainLooper().also {
    it.mediaSourceFor = { FakeMediaSource() }
    it.renderersFactory = fakeRenderersFactory()
    it.clock = FakeClock(true)
  }

  private fun idle() = shadowOf(Looper.getMainLooper()).idle()

  @After
  fun release() { factory.release(); idle() }

  @Test
  fun inv_native_34_playerSnapshotFollowsPlayPauseMuteSeekAndRate() {
    val slot = factory.createSlot(0u)
    assertNull(factory.players.snapshot("k"))

    slot.load("k", route); idle()
    val loaded = factory.players.snapshot("k")!!
    assertEquals("k", loaded.key)
    assertFalse(loaded.isPlaying)
    assertTrue(loaded.isMuted)
    assertEquals(0L, loaded.positionMs)
    assertEquals(1f, loaded.rate, 0f)

    slot.play(); idle()
    assertTrue(factory.players.snapshot("k")!!.isPlaying)
    slot.pause(); idle()
    assertFalse(factory.players.snapshot("k")!!.isPlaying)
    slot.unmute(); idle()
    assertFalse(factory.players.snapshot("k")!!.isMuted)
    slot.mute(); idle()
    assertTrue(factory.players.snapshot("k")!!.isMuted)
    slot.setRate(2f); idle()
    assertEquals(2f, factory.players.snapshot("k")!!.rate, 0f)
    slot.seek(1500uL); idle()
    assertEquals(1500L, factory.players.snapshot("k")!!.positionMs)

    slot.unload(); idle()
    assertNull(factory.players.snapshot("k"))
  }
}
