package com.noma4i.medialayer

import com.noma4i.medialayer.testing.TestFetcher
import com.noma4i.medialayer.CreatedPlayer
import com.noma4i.medialayer.SlotCall
import java.nio.file.Files
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertTrue
import org.junit.Test

private class ReloadSlot : PlayerSlot, PlayerSlotFactory {
  val plays = Channel<Boolean>(Channel.UNLIMITED)
  @Volatile private var attached = false
  @Volatile var generation: ULong = 0uL
  override suspend fun create(slot: UInt): CreatedPlayer = CreatedPlayer(this)
  override suspend fun apply(command: SlotCall) {
    when (command) {
      is SlotCall.Load -> generation = command.generation
      SlotCall.Unload, SlotCall.Detach -> attached = false
      SlotCall.Play -> plays.trySend(attached)
      is SlotCall.Attach -> attached = command.viewToken == 42uL
      else -> {}
    }
  }
  override suspend fun dispose() {}
  override fun sample() = SlotSample(0u, null, false, attached, false)
}

private class ReloadSettings : SettingsStore {
  override fun read(key: String): String? = null
  override fun write(key: String, value: String) {}
  override fun remove(key: String) {}
}

class ReloadTest {
  init { NativeCallbacks.install() }

  @Test
  fun inv_api_06_reload_restores_the_attached_view_before_playing() = runBlocking {
    val slot = ReloadSlot()
    val media = MediaLayer.open(OpenConfig(availableBytes = ULong.MAX_VALUE,
      dir = Files.createTempDirectory("reload").toString(),
      fetcher = TestFetcher(), settings = ReloadSettings(), slots = slot,
      device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings()
    ))
    val key = "file:///clip.mp4"
    val events = media.playerEvents()
    media.attachView(key, 42u)
    media.replaceWindow(0u, listOf(MediaItem(Kind.VIDEO, key, emptyMap(), null, null, strategy = CacheStrategy.WINDOW)))
    media.setPlaying(key, true)
    withTimeout(10_000) {
      assertTrue(slot.plays.receive())
      events.failed(0u, slot.generation, 0, "decoder failed")
      assertTrue(slot.plays.receive())
    }
  }
}
