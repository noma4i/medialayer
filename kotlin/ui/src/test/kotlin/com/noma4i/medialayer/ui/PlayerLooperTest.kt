@file:Suppress("DEPRECATION")

package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.SurfaceTexture
import android.os.Looper
import android.view.Surface
import androidx.media3.common.util.Clock
import androidx.media3.exoplayer.Renderer
import androidx.media3.exoplayer.RenderersFactory
import androidx.media3.test.utils.FakeMediaSource
import androidx.media3.test.utils.FakeVideoRenderer
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.PlaybackRoute
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import kotlin.concurrent.thread
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

@RunWith(MediaLayerTestRunner::class)
class PlayerLooperTest {
  private class GatedVideoRenderer(handler: android.os.Handler, listener: androidx.media3.exoplayer.video.VideoRendererEventListener) :
    FakeVideoRenderer(Clock.DEFAULT.createHandler(handler.looper, null), listener) {
    val attached = CountDownLatch(1)
    val reached = CountDownLatch(1)
    val gate = CountDownLatch(1)
    val acknowledged = CountDownLatch(1)
    val clears = AtomicInteger()
    @Volatile var surface: Surface? = null
    @Volatile var validAtClear: Boolean? = null

    override fun handleMessage(messageType: Int, message: Any?) {
      if (messageType != Renderer.MSG_SET_VIDEO_OUTPUT) return super.handleMessage(messageType, message)
      if (message != null) {
        surface = message as Surface
        attached.countDown()
        return
      }
      reached.countDown()
      gate.await(10, TimeUnit.SECONDS)
      validAtClear = surface?.isValid
      clears.incrementAndGet()
      acknowledged.countDown()
    }
  }

  @Test
  fun inv_output_06_a_video_output_change_does_not_block_the_main_thread() {
    val context = ApplicationProvider.getApplicationContext<Context>()
    val mainWaits = AtomicInteger()
    val countingClock = object : Clock by Clock.DEFAULT {
      override fun onThreadBlocked() {
        if (Looper.myLooper() == Looper.getMainLooper()) mainWaits.incrementAndGet()
      }
    }
    var renderer: GatedVideoRenderer? = null
    val factory = ExoPlayerSlotFactory(context, startMuted = true).also {
      it.mediaSourceFor = { FakeMediaSource() }
      it.clock = countingClock
      it.renderersFactory = RenderersFactory { handler, videoListener, _, _, _ ->
        renderer = GatedVideoRenderer(handler, videoListener)
        arrayOf<Renderer>(renderer!!)
      }
    }
    val releaser = thread(start = false) {
      renderer!!.reached.await(10, TimeUnit.SECONDS)
      renderer!!.gate.countDown()
    }
    val surface = Surface(SurfaceTexture(0))
    try {
      val slot = factory.create(0u)
      val view = factory.players.newView()
      slot.load("k", PlaybackRoute("http://127.0.0.1:9/a.mp4", emptyMap(), null, false, true))
      slot.attach(view.token)
      untilMain { view.player.value != null }
      val output = view.player.value!!
      output.setVideoSurface(surface)
      assertTrue(renderer!!.attached.await(10, TimeUnit.SECONDS))
      releaser.start()

      mainWaits.set(0)
      output.clearVideoSurface(surface)
      val waits = mainWaits.get()

      untilMain { renderer!!.acknowledged.count == 0L }
      assertEquals("the main thread waited for the player to confirm the video output change", 0, waits)
      assertEquals(1, renderer!!.clears.get())
      assertEquals("the surface was released before the player confirmed the change", true, renderer!!.validAtClear)
    } finally {
      renderer?.gate?.countDown()
      factory.release()
      surface.release()
    }
  }

  private fun untilMain(condition: () -> Boolean) {
    val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(10)
    while (!condition()) {
      assertTrue("timed out", System.nanoTime() < deadline)
      shadowOf(Looper.getMainLooper()).idle()
      Thread.sleep(5)
    }
  }
}
