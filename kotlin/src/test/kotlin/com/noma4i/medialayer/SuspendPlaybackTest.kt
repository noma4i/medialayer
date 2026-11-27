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

private class TransportSlot : PlayerSlot, PlayerSlotFactory {
  val log = Channel<String>(Channel.UNLIMITED)
  // A real player reports Ready for every Load; without it the core treats a resumed play as a never-ready preload and reloads it.
  @Volatile var events: PlayerEvents? = null
  private var id = 0u
  override suspend fun create(slot: UInt): CreatedPlayer { id = slot; return CreatedPlayer(this) }
  override suspend fun apply(command: SlotCall) {
    when (command) {
      is SlotCall.Load -> events?.ready(id, command.generation)
      SlotCall.Play -> log.trySend("play")
      SlotCall.Pause -> log.trySend("pause")
      is SlotCall.Rate -> log.trySend("rate")
      else -> {}
    }
  }
  override suspend fun dispose() {}
  override fun sample() = SlotSample(0u, null, false, false, false)
}

private class QuietSettings : SettingsStore {
  override fun read(key: String): String? = null
  override fun write(key: String, value: String) {}
  override fun remove(key: String) {}
}

private const val CLIP = "file:///clip.mp4"

private fun open(slot: TransportSlot?) = MediaLayer.open(OpenConfig(
      audioFocus = com.noma4i.medialayer.AudioFocus.EXCLUSIVE,
  dir = Files.createTempDirectory("suspend").toString(),
  fetcher = TestFetcher(), settings = QuietSettings(), slots = slot,
  device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings()
))

private suspend fun openPlaying(slot: TransportSlot, groups: List<String?> = listOf(null)): MediaLayer {
  val media = open(slot)
  slot.events = media.playerEvents()
  media.replaceWindow(0u, listOf(MediaItem(Kind.VIDEO, CLIP, emptyMap(), null, null, strategy = CacheStrategy.WINDOW)))
  media.setPlaying(CLIP, true, groups)
  assertEquals("play", withTimeout(10_000) { slot.log.receive() })
  return media
}

class SuspendPlaybackTest {
  init { NativeCallbacks.install() }

  @Test
  fun aSuspensionReleasedByHandOrClosedResumesThePlayback() = runBlocking {
    val slot = TransportSlot()
    openPlaying(slot).use { media ->
      val released = media.suspendPlayback()
      assertEquals("pause", withTimeout(10_000) { slot.log.receive() })
      released.release()
      assertEquals("play", withTimeout(10_000) { slot.log.receive() })

      val closed = media.suspendPlayback()
      assertEquals("pause", withTimeout(10_000) { slot.log.receive() })
      closed.close()
      assertEquals("play", withTimeout(10_000) { slot.log.receive() })
    }
  }

  @Test
  fun theAllowedGroupsOfASuspensionReachTheCore() = runBlocking {
    val slot = TransportSlot()
    openPlaying(slot, listOf("chat")).use { media ->
      val allowing = media.suspendPlayback(listOf("chat"))
      media.setRate(CLIP, 2f)
      assertEquals("rate", withTimeout(10_000) { slot.log.receive() })

      val blocking = media.suspendPlayback(listOf("other"))
      assertEquals("pause", withTimeout(10_000) { slot.log.receive() })
      blocking.release()
      assertEquals("play", withTimeout(10_000) { slot.log.receive() })
      allowing.release()
    }
  }
}
