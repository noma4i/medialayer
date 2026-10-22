package com.noma4i.medialayer.ui

import kotlinx.coroutines.CompletableDeferred
import android.content.Context
import android.os.Looper
import android.view.TextureView
import android.view.ViewGroup
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import androidx.core.view.descendants
import androidx.lifecycle.Lifecycle
import androidx.media3.test.utils.FakeMediaSource
import androidx.media3.test.utils.FakeClock
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.PlaybackRoute
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotSame
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

private data class LifecycleSource(override val url: String, override val kind: Kind) : MediaSource {
  override val headers: Map<String, String> get() = emptyMap()
}

@RunWith(MediaLayerTestRunner::class)
class NativeViewLifecycleTest {
  @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()
  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val layer = RecordingLayer()
  private val counters = VideoOutputCounters()
  private val factory = ExoPlayerSlotFactory(context, startMuted = true, videoOutputCounters = counters).also {
    it.mediaSourceFor = { FakeMediaSource() }
    it.bind(layer.playerEvents())
  }
  private val loader = mediaImageLoader(context, ImageBytes { _, _ -> com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK) })
  private val looper = shadowOf(Looper.getMainLooper())

  @Test
  @Config(sdk = [30, 31, 34])
  fun inv_output_05_texture_requires_a_player_to_create_its_view() {
    val events = layer.playerEvents() as RecordingEvents
    val root = rule.activity.findViewById<ViewGroup>(android.R.id.content)
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, factory.players) {
        MediaVideo(LifecycleSource("first", Kind.VIDEO), modifier = Modifier.size(80.dp))
      }
    }
    rule.waitUntil(5000) { looper.idle(); events.calls.any { it.startsWith("attach:first:") } }
    rule.runOnIdle { assertEquals(0, root.descendants.count { it is TextureView }) }
    val token = events.calls.last { it.startsWith("attach:first:") }.substringAfterLast(':').toULong()
    val slot = factory.create(0u)
    slot.load("first", PlaybackRoute("file:///bundle/first.mp4", emptyMap(), null, false, false))
    slot.play()
    slot.attach(token)
    rule.waitUntil(5000) { looper.idle(); counters.snapshot()["setVideoTextureView"] == 1L }
    rule.runOnIdle { assertEquals(1, root.descendants.count { it is TextureView }) }
    slot.detach()
    looper.idle()
    rule.waitForIdle()
    rule.runOnIdle { assertEquals(0, root.descendants.count { it is TextureView }) }
  }

  @Test
  @Config(sdk = [34])
  fun inv_output_05_window_keeps_its_view_during_player_handover() {
    var muted by mutableStateOf(false)
    val events = layer.playerEvents() as RecordingEvents
    val slot = factory.create(0u)
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, factory.players) {
        MediaVideo(LifecycleSource("first", Kind.VIDEO), muted = muted,
          output = MediaOutput.Window, cornerRadius = 16.dp, modifier = Modifier.size(80.dp))
      }
    }
    rule.waitUntil(5000) { looper.idle(); events.calls.any { it.startsWith("attach:first:") } }
    val token = events.calls.last { it.startsWith("attach:first:") }.substringAfterLast(':').toULong()
    slot.load("first", PlaybackRoute("file:///bundle/first.mp4", emptyMap(), null, false, false))
    slot.play()
    slot.attach(token)
    rule.waitUntil(5000) { looper.idle(); counters.snapshot()["setVideoSurfaceView"] == 1L }
    val before = counters.snapshot()
    rule.runOnIdle { muted = true }
    rule.waitForIdle()
    assertEquals(before, counters.snapshot())
    slot.detach()
    looper.idle()
    rule.waitForIdle()
    slot.attach(token)
    rule.waitUntil(5000) { looper.idle(); counters.snapshot()["setVideoSurfaceView"] == 2L }
    val counts = counters.snapshot()
    assertEquals(1L, counts["surfaceCreate"])
    assertEquals(1L, counts["liveSurfaces"])
    assertEquals(1L, counts["create"])
    assertEquals(1L, counts["prepare"])
    assertEquals(null, counts["setVideoTextureView"])
  }

  @Test
  fun inv_native_06_ten_page_changes_do_not_rebind_a_stable_player() {
    var page by mutableStateOf(0)
    var muted by mutableStateOf(false)
    val slots = (0..2).map { factory.create(it.toUInt()) }
    val events = layer.playerEvents() as RecordingEvents
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, factory.players) {
        MediaVideo(LifecycleSource("video$page", Kind.VIDEO), muted = muted, modifier = Modifier.size(80.dp))
      }
    }
    for (next in 0..10) {
      if (next > 0) {
        slots[(next - 1) % 3].detach()
        slots[(next - 1) % 3].pause()
      }
      rule.runOnIdle { page = next }
      val key = "video$next"
      rule.waitUntil(5000) { looper.idle(); events.calls.any { it.startsWith("attach:$key:") } }
      val token = events.calls.last { it.startsWith("attach:$key:") }.substringAfterLast(':').toULong()
      val slot = slots[next % 3]
      slot.load(key, PlaybackRoute("file:///bundle/$next.mp4", emptyMap(), null, false, false))
      slot.play()
      slot.attach(token)
      rule.waitUntil(5000) { looper.idle(); counters.snapshot()["setVideoTextureView"] == (next + 1).toLong() }
      val before = counters.snapshot()
      rule.runOnIdle { muted = !muted }
      rule.waitForIdle()
      assertEquals(before, counters.snapshot())
      assertEquals(1L, counters.snapshot()["liveTextures"])
    }
    val counts = counters.snapshot()
    println("video-output $counts")
    assertEquals(3L, counts["create"])
    assertEquals(3L, counts["prepare"])
    assertEquals(11L, counts["attach"])
    assertEquals(null, counts["repeatAttach"])
    assertEquals(11L, counts["setVideoTextureView"])
  }

  @After
  fun release() {
    rule.activityRule.scenario.moveToState(Lifecycle.State.DESTROYED)
    factory.release()
    loader.shutdown()
    looper.idle()
  }

  @Test
  fun inv_idle_05_paused_video_does_not_repeat_progress_and_seek_resumes_it() {
    var reports = 0
    val slot = factory.create(0u)
    slot.load("first", PlaybackRoute("https://o.test/a.mp4", emptyMap(), null, false, false))
    slot.play()
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, factory.players) {
        Column {
          repeat(2) { MediaVideo(LifecycleSource("first", Kind.VIDEO), modifier = Modifier.size(80.dp), onProgress = { reports++ }) }
        }
      }
    }
    rule.waitUntil(5000) { looper.idle(); reports >= 2 }
    slot.pause()
    looper.idle()
    rule.waitForIdle()
    val paused = reports
    val samples = PlayerProgress.sampleCount
    assertEquals(1, PlayerProgress.producerCount)
    rule.mainClock.advanceTimeBy(500)
    looper.idleFor(java.time.Duration.ofMillis(500))
    assertEquals(paused, reports)
    assertEquals(samples, PlayerProgress.sampleCount)
    slot.seek(200uL)
    rule.waitUntil(5000) { looper.idle(); reports > paused }
  }

  @Test
  fun inv_native_04_the_playing_owner_hands_the_view_back_when_it_leaves() {
    var newer by mutableStateOf(false)
    val events = layer.playerEvents() as RecordingEvents
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, factory.players) {
        Column {
          MediaVideo(LifecycleSource("first", Kind.VIDEO), modifier = Modifier.size(80.dp))
          if (newer) MediaVideo(LifecycleSource("first", Kind.VIDEO), modifier = Modifier.size(80.dp))
        }
      }
    }
    rule.waitUntil(5000) { looper.idle(); events.calls.any { it.startsWith("attach:first:") } }
    val first = events.calls.last { it.startsWith("attach:first:") }
    rule.runOnIdle { newer = true }
    rule.waitUntil(5000) { looper.idle(); events.calls.lastOrNull() != first && factory.players.liveViews == 2 }
    rule.runOnIdle { newer = false }
    rule.waitUntil(5000) { looper.idle(); factory.players.liveViews == 1 }
    assertEquals(first, events.calls.last())
  }

  @Test
  fun inv_native_02_video_defaults_to_playing() {
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, factory.players) {
        MediaVideo(LifecycleSource("first", Kind.VIDEO), modifier = Modifier.size(80.dp))
      }
    }
    rule.waitUntil(5000) { looper.idle(); layer.calls.contains("playing:first:true") }
    assertTrue(layer.calls.contains("playing:first:true"))
  }

  @Test
  fun inv_native_03_video_url_replacement_releases_claim_and_attachment() {
    var url by mutableStateOf("first")
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, factory.players) {
        MediaVideo(LifecycleSource(url, Kind.VIDEO), modifier = Modifier.size(80.dp))
      }
    }
    rule.waitUntil(5000) { looper.idle(); layer.calls.contains("visible:first:true") }
    rule.runOnIdle { url = "second" }
    rule.waitUntil(5000) { looper.idle(); layer.calls.contains("visible:second:true") }
    assertTrue(layer.calls.contains("visible:first:false"))
    val events = layer.playerEvents() as RecordingEvents
    assertTrue(events.calls.contains("detach:first"))
    assertTrue(events.calls.any { it.startsWith("attach:second:") })
  }

  @Test
  fun inv_native_03_audio_url_replacement_replaces_playback_and_claim() {
    var url by mutableStateOf("first")
    var control: AudioControl? = null
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, factory.players) {
        MediaAudio(LifecycleSource(url, Kind.AUDIO)) { control = it; Box(Modifier.size(80.dp)) }
      }
    }
    rule.waitUntil(5000) { looper.idle(); layer.calls.contains("visible:first:true") }
    val previous = control
    rule.runOnIdle { url = "second" }
    rule.waitUntil(5000) { looper.idle(); layer.calls.contains("visible:second:true") }
    assertNotSame(previous, control)
    assertTrue(layer.calls.contains("visible:first:false"))
  }

  @Test
  fun inv_native_05_audio_progress_stops_when_inactive_and_resumes() {
    val clock = FakeClock(0)
    factory.clock = clock
    val slot = factory.create(0u)
    slot.load("first", PlaybackRoute("https://o.test/audio", emptyMap(), null, false, false))
    slot.play()
    var control: AudioControl? = null
    var offset by mutableStateOf(0.dp)
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, factory.players) {
        Box(Modifier.offset(y = offset)) {
          MediaAudio(LifecycleSource("first", Kind.AUDIO)) { control = it; Box(Modifier.size(80.dp)) }
        }
      }
    }
    rule.waitUntil(5000) { clock.advanceTime(50); looper.idle(); slot.player?.isPlaying == true && control != null }
    rule.runOnIdle { control!!.play() }
    clock.advanceTime(1000)
    rule.waitUntil(5000) { looper.idleFor(java.time.Duration.ofMillis(100)); control!!.progress > 0 }
    rule.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
    rule.waitForIdle()
    val background = control!!.progress
    clock.advanceTime(1000)
    repeat(20) { looper.idle(); Thread.sleep(25) }
    assertEquals(background, control!!.progress)
    rule.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
    rule.waitUntil(5000) { looper.idleFor(java.time.Duration.ofMillis(100)); control!!.progress > background }
    rule.runOnIdle { offset = 10000.dp }
    rule.waitForIdle()
    val offscreen = control!!.progress
    clock.advanceTime(1000)
    repeat(20) { looper.idle(); Thread.sleep(25) }
    assertEquals(offscreen, control!!.progress)
    rule.runOnIdle { offset = 0.dp }
    rule.waitUntil(5000) { looper.idleFor(java.time.Duration.ofMillis(100)); control!!.progress > offscreen }
  }

  @Test
  fun inv_audio_01_only_the_ended_state_of_the_core_stops_the_audio() {
    val clock = FakeClock(0)
    factory.clock = clock
    val slot = factory.create(0u)
    slot.load("first", PlaybackRoute("https://o.test/audio", emptyMap(), null, false, false))
    slot.play()
    var control: AudioControl? = null
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, factory.players) {
        MediaAudio(LifecycleSource("first", Kind.AUDIO)) { control = it; Box(Modifier.size(80.dp)) }
      }
    }
    rule.waitUntil(5000) { clock.advanceTime(50); looper.idle(); slot.player?.isPlaying == true && control != null }
    rule.runOnIdle { control!!.play() }

    rule.waitUntil(30000) { clock.advanceTime(1000); looper.idle(); slot.player!!.playbackState == androidx.media3.common.Player.STATE_ENDED }
    rule.waitForIdle()

    assertEquals(androidx.media3.common.Player.STATE_ENDED, slot.player!!.playbackState)
    assertTrue("the player ended and stopped the control without the core", control!!.isPlaying)
    layer.emit("first", com.noma4i.medialayer.MediaState.Ended)
    rule.waitUntil(5000) { looper.idle(); !control!!.isPlaying }
  }

  @Test
  fun inv_native_05_video_progress_stops_when_inactive_and_resumes() {
    val clock = FakeClock(0)
    factory.clock = clock
    val slot = factory.create(0u)
    slot.load("first", PlaybackRoute("https://o.test/video", emptyMap(), null, false, true))
    slot.play()
    var offset by mutableStateOf(0.dp)
    var reports = 0
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, factory.players) {
        MediaVideo(LifecycleSource("first", Kind.VIDEO), modifier = Modifier.offset(y = offset).size(80.dp), onProgress = { reports++ })
      }
    }
    rule.waitUntil(5000) { clock.advanceTime(50); looper.idle(); slot.sample().durationMs != null && reports >= 2 }
    rule.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
    rule.waitForIdle()
    val background = reports
    repeat(20) { rule.mainClock.advanceTimeBy(25); looper.idle() }
    assertEquals(background, reports)
    rule.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
    rule.waitUntil(5000) { looper.idle(); reports > background }
    rule.runOnIdle { offset = 10000.dp }
    rule.waitForIdle()
    val offscreen = reports
    repeat(20) { rule.mainClock.advanceTimeBy(25); looper.idle() }
    assertEquals(offscreen, reports)
    rule.runOnIdle { offset = 0.dp }
    rule.waitUntil(5000) { looper.idle(); reports > offscreen }
  }
}
