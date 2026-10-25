package com.noma4i.medialayer.ui

import android.content.Context
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.*
import java.nio.file.Files
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.CompletableDeferred
import com.noma4i.medialayer.MediaState
import com.noma4i.medialayer.isFailed
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotSame
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private data class InputSource(override val url: String, override val kind: Kind) : MediaSource {
  override val headers: Map<String, String> get() = emptyMap()
}

private class InputSlot : PlayerSlotFactory {
  private data class Event(val slot: UInt, val name: String, val key: String)

  private val events = CopyOnWriteArrayList<Event>()
  val all: List<String> get() = events.map { "${it.slot}:${it.name}:${it.key}" }

  fun playing(key: String): Boolean {
    val loaded = mutableMapOf<UInt, String>()
    val active = mutableSetOf<UInt>()
    for (event in events) when (event.name) {
      "load" -> { loaded[event.slot] = event.key; active.remove(event.slot) }
      "play" -> active.add(event.slot)
      "pause", "unload" -> active.remove(event.slot)
    }
    return active.any { loaded[it] == key }
  }

  fun count(name: String) = events.count { it.name == name }

  override suspend fun create(slot: UInt): CreatedPlayer = CreatedPlayer(Recorder(slot))

  private inner class Recorder(private val slot: UInt) : PlayerSlot {
    @Volatile private var key = ""
    private fun note(name: String) { events.add(Event(slot, name, key)) }
    override suspend fun apply(command: SlotCall) {
      when (command) {
        is SlotCall.Load -> { key = command.key; note("load") }
        SlotCall.Unload -> note("unload")
        SlotCall.Play -> note("play")
        SlotCall.Pause -> note("pause")
        is SlotCall.Rate -> note("rate")
        else -> {}
      }
    }
    override suspend fun dispose() {}
    override fun sample() = SlotSample(0u, null, false, false, false)
  }
}

@RunWith(MediaLayerTestRunner::class)
class MediaAudioInputTest {
  @Test
  fun inv_audio_03_an_ended_control_stops_at_the_start_and_keeps_both_the_toggle_and_the_controlled_input() {
    val control = AudioControl(true) { }
    control.progress = 1f

    control.ended()
    assertFalse(control.isPlaying)
    assertEquals(0f, control.progress)
    control.seek(0.5f)
    assertEquals(0.5f, control.progress)
    control.toggle()
    assertTrue(control.isPlaying)

    control.ended()
    listOf(false, true, false).forEach { control.follow(it); assertEquals(it, control.isPlaying) }
  }

  @Test
  fun inv_audio_07_the_audio_state_table_decides_sounding_buffering_and_failed() {
    val table = listOf(
      Triple(MediaState.Idle, false, true), Triple(MediaState.Loading, false, true), Triple(MediaState.Ready, true, false),
      Triple(MediaState.Rendered, true, false), Triple(MediaState.Ended, false, false), Triple(MediaState.Failed("x"), false, false)
    )
    table.forEach { (state, sounding, buffering) ->
      val control = AudioControl(true) { }
      control.state = state
      assertEquals("$state", sounding, control.isSounding)
      assertEquals("$state", buffering, control.isBuffering)
      assertEquals("$state", state is MediaState.Failed, state.isFailed)
    }
    val paused = AudioControl(false) { }
    paused.state = MediaState.Rendered
    assertFalse(paused.isSounding || paused.isBuffering)
  }

  @Test
  fun inv_audio_06_an_ended_control_rewinds_the_place_to_the_start() {
    val seeks = mutableListOf<Double>()
    val control = AudioControl(true) { seeks.add(it) }
    control.progress = 0.7f

    control.ended()

    assertEquals(listOf(0.0), seeks)
    assertEquals(0f, control.progress)
    assertFalse(control.isPlaying)
  }

  @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()
  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val slot = InputSlot()
  private val loader = mediaImageLoader(context, ImageBytes { _, _ -> com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK) })
  private val layer = MediaLayer.open(OpenConfig(availableBytes = ULong.MAX_VALUE,
    dir = Files.createTempDirectory("audio-input").toString(),
    fetcher = object : Fetcher {
      override suspend fun fetch(request: FetchRequest): FetchResponse = throw MediaException.Fetch("unused origin")
    },
    settings = SharedPreferencesSettings(context, "audio-input"), slots = slot,
    device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings()
  ))

  private val looper = shadowOf(Looper.getMainLooper())
  private val seen = CopyOnWriteArrayList<AudioControl>()
  private var firstSeenPlaying: Boolean? = null
  private var playOnMount = false
  private var url by mutableStateOf("file:///first.mp4")
  private var playing by mutableStateOf(false)
  private var rate by mutableStateOf(1f)
  private var muted by mutableStateOf(false)
  private var group by mutableStateOf("ring")
  private var unrelated by mutableIntStateOf(0)

  private fun waitForPlaying(key: String, expected: Boolean = true) = rule.waitUntil(10_000) { looper.idle(); slot.playing(key) == expected }

  private fun mount(): AudioControl {
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, null) {
        val tick = unrelated
        MediaAudio(InputSource(url, Kind.AUDIO), playing = playing, muted = muted, rate = rate, group = group) { control ->
          if (firstSeenPlaying == null) firstSeenPlaying = control.isPlaying
          seen.add(control)
          if (playOnMount) {
            playOnMount = false
            control.play()
          }
          Box(Modifier.size(80.dp + tick.dp))
        }
      }
    }
    rule.waitUntil(10_000) { seen.isNotEmpty() }
    return seen.last()
  }

  // Compose is idle, then a new rate of the mounted audio reaches the slot behind every earlier command of its place.
  private fun barrier() {
    rule.waitForIdle()
    looper.idle()
    val before = slot.count("rate")
    rule.runOnIdle { rate = 3f + before }
    rule.waitUntil(10_000) { looper.idle(); slot.count("rate") > before }
  }

  @Test
  fun a_mounted_audio_follows_a_changing_playing_input_without_remount() {
    val first = mount()
    rule.runOnIdle { assertFalse(first.isPlaying) }
    assertEquals(0, slot.count("play"))

    rule.runOnIdle { playing = true }
    waitForPlaying("file:///first.mp4")
    rule.runOnIdle { assertTrue(first.isPlaying) }

    rule.runOnIdle { playing = false }
    waitForPlaying("file:///first.mp4", false)
    rule.runOnIdle { assertFalse(first.isPlaying) }
    seen.forEach { assertSame(first, it) }
  }

  @Test
  fun a_mounted_audio_starts_at_an_initial_true_playing_input() {
    playing = true
    val first = mount()
    assertEquals(true, firstSeenPlaying)
    waitForPlaying("file:///first.mp4")
    rule.runOnIdle { assertTrue(first.isPlaying) }
  }

  @Test
  fun a_local_play_at_mount_survives_an_initial_false_playing_input() {
    playOnMount = true
    val first = mount()
    waitForPlaying("file:///first.mp4")
    barrier()
    rule.runOnIdle { assertTrue(first.isPlaying) }
    assertTrue(slot.playing("file:///first.mp4"))
  }

  @Test
  fun a_local_toggle_survives_rerenders_that_leave_playing_unchanged() {
    val first = mount()
    rule.runOnIdle { first.toggle() }
    waitForPlaying("file:///first.mp4")

    rule.runOnIdle {
      unrelated += 1
      rate = 2f
      muted = true
      group = "system"
    }
    barrier()

    rule.runOnIdle { assertTrue(first.isPlaying) }
    assertTrue(slot.playing("file:///first.mp4"))
    assertEquals(0, slot.count("pause"))
    seen.forEach { assertSame(first, it) }
  }

  @Test
  fun a_changed_source_releases_the_old_one_and_starts_from_the_current_playing_input() {
    playing = true
    val first = mount()
    waitForPlaying("file:///first.mp4")

    rule.runOnIdle {
      url = "file:///second.mp4"
      playing = false
    }
    rule.waitUntil(10_000) { looper.idle(); seen.last() !== first }
    rule.runOnIdle { assertFalse(seen.last().isPlaying) }

    rule.runOnIdle {
      url = "file:///third.mp4"
      playing = true
    }
    waitForPlaying("file:///third.mp4")
    assertNotSame(first, seen.last())
    rule.runOnIdle { assertTrue(seen.last().isPlaying) }
    assertFalse(slot.all.any { it.contains(":play:file:///second.mp4") })
    assertFalse(slot.playing("file:///first.mp4"))
  }
}
