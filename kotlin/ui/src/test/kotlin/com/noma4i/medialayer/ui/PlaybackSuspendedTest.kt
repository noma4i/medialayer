package com.noma4i.medialayer.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import com.noma4i.medialayer.testing.TestFetcher
import android.content.Context
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.*
import java.nio.file.Files
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private const val SUSPEND_CLIP = "file:///clip.mp4"

private class SuspendProbeSlot : PlayerSlot, PlayerSlotFactory {
  val transport = CopyOnWriteArrayList<String>()
  override suspend fun create(slot: UInt): CreatedPlayer = CreatedPlayer(this)
  override suspend fun apply(command: SlotCall) {
    when (command) {
      SlotCall.Play -> transport.add("play")
      SlotCall.Pause -> transport.add("pause")
      else -> {}
    }
  }
  override suspend fun dispose() {}
  override fun sample() = SlotSample(0u, null, false, false, false)
}

@RunWith(MediaLayerTestRunner::class)
class PlaybackSuspendedTest {
  @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()
  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val slot = SuspendProbeSlot()
  private val looper = shadowOf(Looper.getMainLooper())
  private val loader = mediaImageLoader(context, ImageBytes { _, _ -> ImageFetchResult(pngBytes(), ImageOrigin.NETWORK) })

  private fun openLayer(ingress: IngressLimits? = null): MediaLayer {
    val layer = MediaLayer.open(OpenConfig(availableBytes = ULong.MAX_VALUE,
      dir = Files.createTempDirectory("suspended").toString(),
      fetcher = TestFetcher(),
      settings = SharedPreferencesSettings(context, "suspended"), slots = slot,
      device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings(), ingress = ingress
    ))
    layer.replaceWindow(0u, listOf(MediaItem(Kind.VIDEO, SUSPEND_CLIP, emptyMap(), null, null, strategy = CacheStrategy.WINDOW)))
    layer.setPlaying(SUSPEND_CLIP, true, listOf(null))
    waitFor(listOf("play"))
    return layer
  }

  private fun waitFor(expected: List<String>) = rule.waitUntil(10_000) { looper.idle(); slot.transport.toList() == expected }

  private fun waitForLast(last: String) = rule.waitUntil(10_000) { looper.idle(); slot.transport.lastOrNull() == last }

  @Test
  fun inv_session_18_a_mounted_suspension_follows_active_and_never_duplicates() {
    val layer = openLayer()
    var active by mutableStateOf(false)
    var groups by mutableStateOf(emptyList<String>())
    var unrelated by mutableStateOf(0)
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, null) {
        unrelated.let { SuspendMediaPlayback(active, groups) }
      }
    }
    rule.runOnIdle { groups = listOf("other") }
    assertEquals(listOf("play"), slot.transport.toList())

    rule.runOnIdle { active = true }
    waitFor(listOf("play", "pause"))
    rule.runOnIdle { active = true; unrelated += 1; groups = listOf("other") }

    rule.runOnIdle { active = false }
    waitFor(listOf("play", "pause", "play"))
    layer.close()
  }

  @Test
  fun inv_session_18_a_repeated_intent_takes_no_second_hold() {
    val layer = openLayer(IngressLimits(256u, 3u, 1UL shl 20))
    val hold = SuspensionHold()
    hold.apply(layer, true, listOf("other"))
    waitFor(listOf("play", "pause"))
    hold.apply(layer, true, listOf("other"))
    assertNull(Media.failure)
    hold.release()
    waitFor(listOf("play", "pause", "play"))
    layer.close()
  }

  @Test
  fun inv_session_18_a_suspension_left_after_a_hold_releases_it() {
    val layer = openLayer()
    var mounted by mutableStateOf(true)
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, null) { if (mounted) SuspendMediaPlayback(active = true) }
    }
    waitFor(listOf("play", "pause"))
    rule.runOnIdle { mounted = false }
    rule.waitUntil(1_500) { looper.idle(); slot.transport.toList() == listOf("play", "pause", "play") }
    layer.close()
  }

  @Test
  fun inv_session_18_a_suspension_left_before_the_layer_opens_holds_nothing() {
    val layer = openLayer()
    var mounted by mutableStateOf(true)
    val opening = CompletableDeferred<MediaLayer>()
    rule.setContent {
      MediaLayerProvider(opening) { if (mounted) SuspendMediaPlayback(active = true) }
    }
    rule.runOnIdle { mounted = false }
    rule.runOnIdle { opening.complete(layer) }
    looper.idle()
    rule.waitForIdle()
    assertEquals(listOf("play"), slot.transport.toList())
    assertNull(Media.failure)
    layer.close()
  }

  @Test
  fun inv_session_19_a_group_change_replaces_the_hold_without_a_gap() {
    val layer = openLayer()
    var active by mutableStateOf(true)
    var groups by mutableStateOf(listOf("one"))
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, null) { SuspendMediaPlayback(active, groups) }
    }
    waitFor(listOf("play", "pause"))
    rule.runOnIdle { groups = listOf("two") }
    rule.waitForIdle()
    looper.idle()
    rule.runOnIdle { active = false }
    waitFor(listOf("play", "pause", "play"))
    layer.close()
    looper.idle()
    assertEquals(listOf("play", "pause", "play"), slot.transport.toList())
  }

  @Test
  fun inv_session_19_a_release_reaches_the_core_through_a_full_input() {
    val layer = openLayer(IngressLimits(256u, 3u, 1UL shl 20))
    var active by mutableStateOf(true)
    var groups by mutableStateOf(listOf("one"))
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, null) { SuspendMediaPlayback(active, groups) }
    }
    waitForLast("pause")
    rule.runOnIdle { groups = listOf("two") }
    rule.waitUntil(10_000) { looper.idle(); Media.failure != null }
    assertEquals(listOf("play", "pause"), slot.transport.toList())
    rule.runOnIdle { active = false }
    waitFor(listOf("play", "pause", "play"))
    Media.failure = null
    layer.close()
  }

  @Test
  fun inv_session_20_a_playing_place_survives_background_and_foreground_without_app_code() {
    val layer = openLayer()
    val owner = object : androidx.lifecycle.LifecycleOwner { override val lifecycle = androidx.lifecycle.LifecycleRegistry(this) }
    val signals = SystemSignals(context, layer.playerEvents(), owner.lifecycle)
    signals.onStop(owner)
    waitFor(listOf("play", "pause"))
    signals.onStart(owner)
    waitForLast("play")
    looper.idle()
    assertEquals(listOf("play", "pause", "play"), slot.transport.toList().fold(emptyList<String>()) { seen, next -> if (seen.lastOrNull() == next) seen else seen + next })
    layer.close()
  }
}
