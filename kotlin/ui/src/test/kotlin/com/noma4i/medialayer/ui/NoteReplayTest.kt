package com.noma4i.medialayer.ui

import android.content.Context
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.*
import com.noma4i.medialayer.testing.TestFetcher
import java.nio.file.Files
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.CompletableDeferred
import org.junit.Assert.assertFalse
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private data class NoteSource(override val url: String, override val kind: Kind) : MediaSource {
  override val headers: Map<String, String> get() = emptyMap()
}

private class NoteSlot : PlayerSlotFactory, PlayerSlot {
  private val events = CopyOnWriteArrayList<String>()
  @Volatile var generation = 0uL
  val all: List<String> get() = events.toList()

  fun count(name: String) = events.count { it == name }

  override suspend fun create(slot: UInt): CreatedPlayer = CreatedPlayer(this)
  override suspend fun apply(command: SlotCall) {
    when (command) {
      is SlotCall.Load -> { generation = command.generation; events.add("load") }
      is SlotCall.Seek -> events.add("seek ${command.positionMs}")
      SlotCall.Unload -> events.add("unload")
      SlotCall.Play -> events.add("play")
      else -> {}
    }
  }
  override suspend fun dispose() {}
  override fun sample() = SlotSample(60_000uL, 60_000uL, false, false, false)
}

@RunWith(MediaLayerTestRunner::class)
class NoteReplayTest {
  @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()
  private val context = ApplicationProvider.getApplicationContext<Context>()
  private val slot = NoteSlot()
  private val loader = mediaImageLoader(context, ImageBytes { _, _ -> ImageFetchResult(pngBytes(), ImageOrigin.NETWORK) })
  private val layer = MediaLayer.open(OpenConfig(
      audioFocus = com.noma4i.medialayer.AudioFocus.EXCLUSIVE,
    dir = Files.createTempDirectory("note-replay").toString(),
    fetcher = TestFetcher(),
    settings = SharedPreferencesSettings(context, "note-replay"), slots = slot,
    device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings()
  ))
  private val looper = shadowOf(Looper.getMainLooper())
  private var shown by mutableStateOf(true)

  private fun until(condition: () -> Boolean) = rule.waitUntil(COMPOSE_WAIT_MS) { looper.idle(); condition() }

  @Test
  fun inv_ingress_44_a_note_that_ended_and_was_unmounted_starts_from_the_start_when_mounted_again() {
    rule.setContent {
      MediaLayerProvider(CompletableDeferred(layer), loader, null) {
        if (shown) MediaAudio(NoteSource("file:///note.m4a", Kind.AUDIO), playing = true) { Box(Modifier.size(80.dp)) }
      }
    }
    until { slot.count("load") == 1 && slot.count("play") == 1 }
    layer.playerEvents().ready(0u, slot.generation)
    layer.playerEvents().ended(0u, slot.generation)
    until { slot.count("seek 0") >= 1 }

    rule.runOnIdle { shown = false }
    until { slot.count("unload") == 1 }
    rule.runOnIdle { shown = true }
    until { slot.count("load") == 2 && slot.count("play") == 2 }

    val events = slot.all
    val load = events.lastIndexOf("load")
    assertFalse("$events", events.drop(load).any { it.startsWith("seek ") && it != "seek 0" })
  }
}
