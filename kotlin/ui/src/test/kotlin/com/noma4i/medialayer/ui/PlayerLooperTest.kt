@file:Suppress("DEPRECATION")

package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.Canvas
import android.graphics.Rect
import android.graphics.SurfaceTexture
import android.view.SurfaceHolder
import android.view.TextureView
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
import java.util.concurrent.CopyOnWriteArrayList
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
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
    var created: ExoPlayerSlot? = null
    try {
      val slot = factory.createSlot(0u).also { created = it }
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
      created?.let { kotlinx.coroutines.runBlocking { it.dispose() } }
      surface.release()
    }
  }

  private class TestHolder(val frame: Surface) : SurfaceHolder {
    val callbacks = CopyOnWriteArrayList<SurfaceHolder.Callback>()
    override fun addCallback(callback: SurfaceHolder.Callback) { callbacks.add(callback) }
    override fun removeCallback(callback: SurfaceHolder.Callback) { callbacks.remove(callback) }
    override fun isCreating() = false
    @Deprecated("test holder") override fun setType(type: Int) = Unit
    override fun setFixedSize(width: Int, height: Int) = Unit
    override fun setSizeFromLayout() = Unit
    override fun setFormat(format: Int) = Unit
    override fun setKeepScreenOn(screenOn: Boolean) = Unit
    override fun lockCanvas(): Canvas? = null
    override fun lockCanvas(dirty: Rect?): Canvas? = null
    override fun unlockCanvasAndPost(canvas: Canvas) = Unit
    override fun getSurface(): Surface = frame
    override fun getSurfaceFrame(): Rect = Rect(0, 0, 0, 0)
  }

  private enum class Loss { LeaveVideo, TextureDestroyed, HolderDestroyed }

  @Test
  fun inv_output_07_video_output_loss_never_calls_the_player_from_the_main_thread() {
    val failures = Loss.values().mapNotNull { loss ->
      try {
        checkLoss(loss)
        null
      } catch (error: AssertionError) {
        "[$loss] ${error.message}"
      }
    }
    assertTrue(failures.joinToString("\n"), failures.isEmpty())
  }

  private fun checkLoss(loss: Loss) {
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
    val checked = CountDownLatch(1)
    val releaser = thread(start = false) {
      renderer!!.reached.await(10, TimeUnit.SECONDS)
      if (loss == Loss.TextureDestroyed) checked.await(1, TimeUnit.SECONDS)
      renderer!!.gate.countDown()
    }
    val frame = Surface(SurfaceTexture(0))
    val released = CountDownLatch(1)
    val texture = object : SurfaceTexture(0) {
      override fun release() {
        released.countDown()
        super.release()
      }
    }
    var created: ExoPlayerSlot? = null
    try {
      val slot = factory.createSlot(0u).also { created = it }
      val view = factory.players.newView()
      slot.load("k", PlaybackRoute("http://127.0.0.1:9/a.mp4", emptyMap(), null, false, true))
      slot.attach(view.token)
      untilMain { view.player.value != null }
      val output = view.player.value!!
      val holder = TestHolder(frame)
      val textureView = TextureView(context)
      when (loss) {
        Loss.LeaveVideo, Loss.HolderDestroyed -> {
          output.setVideoSurfaceHolder(holder)
          if (loss == Loss.HolderDestroyed) untilMain { holder.callbacks.isNotEmpty() }
          assertTrue("$loss: no video output", renderer!!.attached.await(10, TimeUnit.SECONDS))
        }
        Loss.TextureDestroyed -> {
          output.setVideoTextureView(textureView)
          untilMain { textureView.surfaceTextureListener != null }
          textureView.surfaceTextureListener!!.onSurfaceTextureAvailable(texture, 0, 0)
          assertTrue("$loss: no video output", renderer!!.attached.await(10, TimeUnit.SECONDS))
        }
      }
      releaser.start()

      mainWaits.set(0)
      when (loss) {
        Loss.LeaveVideo -> output.clearVideoSurfaceHolder(holder)
        Loss.HolderDestroyed -> {
          holder.callbacks.forEach { it.surfaceDestroyed(holder) }
          assertEquals("$loss: returned before the player confirmed", 0L, renderer!!.acknowledged.count)
        }
        Loss.TextureDestroyed -> {
          val kept = textureView.surfaceTextureListener!!.onSurfaceTextureDestroyed(texture)
          assertFalse("$loss: the texture was handed to the platform for release", kept)
          assertTrue("$loss: the player did not receive the clear", renderer!!.reached.await(10, TimeUnit.SECONDS))
          assertEquals("$loss: texture released before the player confirmed", 1L, released.count)
        }
      }
      val waits = mainWaits.get()
      checked.countDown()

      untilMain { renderer!!.acknowledged.count == 0L }
      if (loss == Loss.TextureDestroyed) assertTrue("$loss: texture never released", released.await(10, TimeUnit.SECONDS))
      assertEquals("$loss: the main thread waited inside the player", 0, waits)
      assertEquals("$loss: clears", 1, renderer!!.clears.get())
      assertEquals("$loss: the output was released before the player confirmed", true, renderer!!.validAtClear)
    } finally {
      checked.countDown()
      renderer?.gate?.countDown()
      factory.release()
      created?.let { kotlinx.coroutines.runBlocking { it.dispose() } }
      frame.release()
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
