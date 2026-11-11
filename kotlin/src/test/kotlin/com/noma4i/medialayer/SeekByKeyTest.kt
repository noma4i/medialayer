package com.noma4i.medialayer

import com.noma4i.medialayer.testing.TestFetcher
import com.noma4i.medialayer.CreatedPlayer
import com.noma4i.medialayer.SlotCall
import java.nio.file.Files
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Test

private class SeekSlot : PlayerSlot, PlayerSlotFactory {
  val log = Channel<String>(Channel.UNLIMITED)
  override suspend fun create(slot: UInt): CreatedPlayer = CreatedPlayer(this)
  override suspend fun apply(command: SlotCall) {
    when (command) {
      is SlotCall.Load -> log.trySend("load ${command.generation}")
      is SlotCall.Seek -> log.trySend("seek ${command.positionMs}")
      else -> {}
    }
  }
  override suspend fun dispose() {}
  override fun sample() = SlotSample(0u, 60_000u, false, false, false)
}

private class SeekSettings : SettingsStore {
  override fun read(key: String): String? = null
  override fun write(key: String, value: String) {}
  override fun remove(key: String) {}
}

private const val CLIP = "file:///clip.mp4"

private fun open(slot: SeekSlot) = MediaLayer.open(OpenConfig(
  dir = Files.createTempDirectory("seek").toString(),
  fetcher = TestFetcher(), settings = SeekSettings(), slots = slot,
  device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings()
))

class SeekByKeyTest {
  init { NativeCallbacks.install() }

  @Test
  fun inv_ingress_40_aSeekOfAKeyWithoutAMountedPlaceReachesTheCoreAndAppliesWhenTheKeyLoads() = runBlocking {
    val slot = SeekSlot()
    open(slot).use { media ->
      media.seek(CLIP, 0.5)
      media.replaceWindow(0u, listOf(MediaItem(Kind.VIDEO, CLIP, emptyMap(), null, null, strategy = CacheStrategy.WINDOW)))
      media.setPlaying(CLIP, true, listOf(null))

      withTimeout(10_000) {
        val loaded = slot.log.receive().removePrefix("load ").toULong()
        media.playerEvents().ready(0u, loaded)
        assertEquals("seek 30000", slot.log.receive())
      }
    }
  }
}
