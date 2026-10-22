package com.noma4i.medialayer.ui

import android.content.Context
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.size
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSource
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private data class Backdrop(override val url: String) : MediaSource {
  override val kind: Kind get() = Kind.IMAGE
  override val headers: Map<String, String> get() = emptyMap()
}

@RunWith(MediaLayerTestRunner::class)
class MediaBackgroundTest {
  @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val layer = RecordingLayer()
  private val slots = ExoPlayerSlotFactory(context, startMuted = true).onMainLooper()
  private val looper = shadowOf(android.os.Looper.getMainLooper())
  private val started = AtomicInteger()
  private val cancelled = AtomicInteger()
  private val gate = CompletableDeferred<Unit>()
  private val url = "https://o.test/back.gif"
  private val loader by lazy {
    mediaImageLoader(context, ImageBytes { _, _ ->
      started.incrementAndGet()
      try { gate.await() } catch (e: CancellationException) { cancelled.incrementAndGet(); throw e }
      com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK)
    }).newBuilder().allowHardware(false).build()
  }

  private fun claims() = layer.calls.filter { it.startsWith("visible:$url:") }

  private fun settle() = repeat(20) { looper.idle(); Thread.sleep(25) }

  @Test
  fun aStoppedActivityReleasesTheKeyAndCancelsTheRequestAndARestartAsksAgain() {
    rule.setContent { MediaLayerProvider(CompletableDeferred(layer), loader, slots.players) { MediaImage(Backdrop(url), Modifier.size(50.dp)) } }
    rule.waitUntil(5000) { looper.idle(); started.get() == 1 }

    rule.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
    rule.waitForIdle()
    settle()
    assertEquals("started=1 cancelled=1 claims=[visible:$url:true, visible:$url:false]", "started=${started.get()} cancelled=${cancelled.get()} claims=${claims()}")

    rule.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
    rule.waitForIdle()
    settle()
    gate.complete(Unit)
    rule.waitUntil(5000) { looper.idle(); layer.calls.any { it == "itemState:$url:Ready" } }

    assertEquals("started=2 cancelled=1 claims=[visible:$url:true, visible:$url:false, visible:$url:true]", "started=${started.get()} cancelled=${cancelled.get()} claims=${claims()}")
  }
}
