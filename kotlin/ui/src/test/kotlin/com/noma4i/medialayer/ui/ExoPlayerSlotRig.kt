@file:Suppress("DEPRECATION")

package com.noma4i.medialayer.ui

import android.content.Context
import android.os.Looper
import androidx.media3.test.utils.FakeClock
import androidx.media3.test.utils.FakeMediaSource
import androidx.test.core.app.ApplicationProvider
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import com.noma4i.medialayer.PlaybackRoute
import okhttp3.OkHttpClient
import org.junit.After
import org.robolectric.Shadows.shadowOf

@OptIn(ExperimentalCoroutinesApi::class)
internal abstract class ExoPlayerSlotRig {
  protected val context = ApplicationProvider.getApplicationContext<Context>()
  protected val events = RecordingEvents()
  protected val route = PlaybackRoute("http://127.0.0.1:9/a.mp4", emptyMap(), null, false, true)
  protected val factories = mutableListOf<ExoPlayerSlotFactory>()

  protected val focusPort = FakeFocusPort()
  protected val focusScope = TestScope()

  protected fun session(focus: MediaAudioFocus) = MediaAudioSession(focus, focusPort, 300, focusScope)

  protected fun settleAudioFocus() = repeat(20) { idle(); Thread.sleep(10) }

  protected fun advanceFocus(ms: Long) {
    focusScope.advanceTimeBy(ms)
    focusScope.runCurrent()
  }

  protected fun factory(startMuted: Boolean = true, focus: MediaAudioFocus = MediaAudioFocus.Mix) =
    ExoPlayerSlotFactory(context, OkHttpClient(), startMuted = startMuted, audio = session(focus)).onMainLooper().also {
      it.mediaSourceFor = { FakeMediaSource() }
      it.bind(events)
      factories.add(it)
    }

  protected fun idle() = shadowOf(Looper.getMainLooper()).idle()

  protected fun disposedSlot(factory: ExoPlayerSlotFactory): java.lang.ref.WeakReference<ExoPlayerSlot> {
    val slot = factory.createSlot(0u)
    slot.load("k", route); idle(); slot.disposing(); idle()
    return java.lang.ref.WeakReference(slot)
  }

  @After
  fun releaseEverything() { factories.forEach { it.release() }; idle() }

  protected fun playUntilReady(clock: FakeClock, slot: ExoPlayerSlot) {
    repeat(100) {
      if (slot.player!!.isPlaying) return
      clock.advanceTime(50); idle(); Thread.sleep(20)
    }
  }

  protected fun untilIdle(condition: () -> Boolean) {
    val deadline = System.nanoTime() + 10_000_000_000L
    while (!condition()) {
      check(System.nanoTime() < deadline) { "condition never held" }
      idle()
      Thread.yield()
    }
  }
}


internal fun fakeRenderersFactory() = androidx.media3.exoplayer.RenderersFactory { handler, videoListener, _, _, _ ->
  arrayOf<androidx.media3.exoplayer.Renderer>(
    androidx.media3.test.utils.FakeVideoRenderer(androidx.media3.common.util.Clock.DEFAULT.createHandler(handler.looper, null), videoListener)
  )
}
