package com.noma4i.medialayer

import com.noma4i.medialayer.testing.TestFetcher
import java.nio.file.Files
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Test

private class OwnerSlot : PlayerSlot, PlayerSlotFactory {
  val calls = Channel<String>(Channel.UNLIMITED)
  override suspend fun create(slot: UInt): CreatedPlayer = CreatedPlayer(this)
  override suspend fun apply(command: SlotCall) {
    when (command) {
      is SlotCall.Load -> calls.trySend("load ${command.key}")
      SlotCall.Unload -> calls.trySend("unload")
      else -> {}
    }
  }
  override suspend fun dispose() {}
  override fun sample() = SlotSample(0u, null, false, false, false)
}

private class OwnerSettings : SettingsStore {
  override fun read(key: String): String? = null
  override fun write(key: String, value: String) {}
  override fun remove(key: String) {}
}

class WindowOwnerTest {
  init { NativeCallbacks.install() }

  private fun item(url: String) = MediaItem(Kind.VIDEO, url, emptyMap(), null, null, strategy = CacheStrategy.WINDOW)

  @Test
  fun inv_ingress_49_a_new_window_without_sources_leaves_the_stage_to_the_window_that_has_them() = runBlocking {
    val slot = OwnerSlot()
    val media = MediaLayer.open(OpenConfig(
      dir = Files.createTempDirectory("owner").toString(),
      fetcher = TestFetcher(), settings = OwnerSettings(), slots = slot,
      device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings()
    ))
    val feed = media.openVideoWindow()
    feed.replace(0u, listOf(item("file:///feed.mp4")))
    val seen = mutableListOf<String>()
    withTimeout(10_000) { while ("load file:///feed.mp4" !in seen) seen.add(slot.calls.receive()) }

    val offscreen = media.openVideoWindow()
    offscreen.replace(0u, emptyList())
    offscreen.moveTo(0u)
    offscreen.setActive(false)
    feed.replace(0u, listOf(item("file:///feed.mp4"), item("file:///next.mp4")))
    withTimeout(10_000) { while ("load file:///next.mp4" !in seen) seen.add(slot.calls.receive()) }

    assertEquals(listOf("load file:///feed.mp4", "load file:///next.mp4"), seen.filter { it.startsWith("load") || it == "unload" })
  }
}
