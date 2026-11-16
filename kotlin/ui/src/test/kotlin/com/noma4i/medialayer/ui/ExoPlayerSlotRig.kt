@file:Suppress("DEPRECATION")

package com.noma4i.medialayer.ui

import android.content.Context
import android.os.Looper
import androidx.media3.test.utils.FakeClock
import androidx.media3.test.utils.FakeMediaSource
import androidx.test.core.app.ApplicationProvider
import kotlinx.coroutines.ExperimentalCoroutinesApi
import com.noma4i.medialayer.PlaybackRoute
import com.noma4i.medialayer.AudioFocus
import com.noma4i.medialayer.DeviceMemory
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.OpenConfig
import com.noma4i.medialayer.defaultLimits
import com.noma4i.medialayer.defaultPoolSettings
import java.nio.file.Files
import okhttp3.OkHttpClient
import org.junit.After
import org.robolectric.Shadows.shadowOf

internal typealias TestFetcher = com.noma4i.medialayer.testing.TestFetcher

internal class AudioFixture(context: Context, focus: MediaAudioFocus = MediaAudioFocus.Mix, val port: RecordingFocusPort = RecordingFocusPort(), volume: VolumePort? = null) : AutoCloseable {
  val core = MediaLayer.open(OpenConfig(
    dir = Files.createTempDirectory("audio-fixture").toString(), fetcher = TestFetcher(),
    settings = SharedPreferencesSettings(context, "audio-fixture-${System.nanoTime()}"), slots = null,
    device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings(),
    audioFocus = when (focus) { MediaAudioFocus.Mix -> AudioFocus.MIX; MediaAudioFocus.Duck -> AudioFocus.DUCK; MediaAudioFocus.Exclusive -> AudioFocus.EXCLUSIVE }
  ))
  val link = AudioLink(core, AudioPorts(port, GrantedPermission), volume)
  override fun close() { link.close(); core.close() }
}

@OptIn(ExperimentalCoroutinesApi::class)
internal abstract class ExoPlayerSlotRig {
  protected val context = ApplicationProvider.getApplicationContext<Context>()
  protected val events = RecordingEvents()
  protected val route = PlaybackRoute("http://127.0.0.1:9/a.mp4", emptyMap(), null, false, true)
  protected val factories = mutableListOf<ExoPlayerSlotFactory>()

  protected val focusPort = RecordingFocusPort()
  private val audioFixtures = mutableListOf<AudioFixture>()

  protected fun settleAudioFocus() = idle()

  protected fun advanceFocus(ms: Long) = idle()

  protected fun factory(startMuted: Boolean = true, focus: MediaAudioFocus = MediaAudioFocus.Mix) =
    ExoPlayerSlotFactory(context, OkHttpClient(), startMuted = startMuted).onMainLooper().also {
      it.clock = FakeClock(true)
      it.renderersFactory = fakeRenderersFactory()
      val fixture = AudioFixture(context, focus, focusPort)
      audioFixtures.add(fixture)
      it.link = fixture.link
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
  fun releaseEverything() { factories.forEach { it.release() }; audioFixtures.forEach { it.close() }; idle() }

  protected fun playUntilReady(clock: FakeClock, slot: ExoPlayerSlot) {
    androidx.media3.test.utils.robolectric.TestPlayerRunHelper.advance(slot.player!!)
      .withTimeoutMs(2_000).untilState(androidx.media3.common.Player.STATE_READY)
  }

  protected fun untilIdle(slot: ExoPlayerSlot, condition: () -> Boolean) {
    TestSignal.await("untilIdle at ${Throwable().stackTrace[1]}", 10_000) {
      idle()
      slot.player?.applicationLooper?.let { shadowOf(it).idle() }
      condition()
    }
  }
}


internal fun fakeRenderersFactory() = androidx.media3.exoplayer.RenderersFactory { handler, videoListener, _, _, _ ->
  arrayOf<androidx.media3.exoplayer.Renderer>(
    androidx.media3.test.utils.FakeVideoRenderer(androidx.media3.common.util.Clock.DEFAULT.createHandler(handler.looper, null), videoListener)
  )
}
