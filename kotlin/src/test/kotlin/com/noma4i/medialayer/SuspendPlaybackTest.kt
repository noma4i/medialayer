package com.noma4i.medialayer

import com.noma4i.medialayer.CreatedPlayer
import com.noma4i.medialayer.SlotCall
import java.nio.file.Files
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Test

private class TransportSlot : PlayerSlot, PlayerSlotFactory {
  val log = Channel<String>(Channel.UNLIMITED)
  override suspend fun create(slot: UInt): CreatedPlayer = CreatedPlayer(this)
  override suspend fun apply(command: SlotCall) {
    when (command) {
      SlotCall.Play -> log.trySend("play")
      SlotCall.Pause -> log.trySend("pause")
      is SlotCall.Rate -> log.trySend("rate")
      else -> {}
    }
  }
  override fun sample() = SlotSample(0u, null, false, false, false)
}

private class QuietSettings : SettingsStore {
  override fun read(key: String): String? = null
  override fun write(key: String, value: String) {}
  override fun remove(key: String) {}
}

private class QuietFetcher : Fetcher {
  override suspend fun fetch(request: FetchRequest): FetchResponse = throw MediaException.Fetch("offline")
}

private const val CLIP = "file:///clip.mp4"

private fun open(slot: TransportSlot?) = MediaLayer.open(OpenConfig(availableBytes = ULong.MAX_VALUE,
  dir = Files.createTempDirectory("suspend").toString(),
  fetcher = QuietFetcher(), settings = QuietSettings(), slots = slot,
  device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings()
))

private suspend fun openPlaying(slot: TransportSlot, groups: List<String?> = listOf(null)): MediaLayer {
  val media = open(slot)
  media.replaceWindow(0u, listOf(MediaItem(Kind.VIDEO, CLIP, emptyMap(), null, null, strategy = CacheStrategy.WINDOW)))
  media.setPlaying(CLIP, true, groups)
  assertEquals("play", withTimeout(10_000) { slot.log.receive() })
  return media
}

class SuspendPlaybackTest {
  init { NativeCallbacks.install() }

  @Test
  fun inv_session_12_the_last_release_resumes_and_the_first_one_does_not() = runBlocking {
    val slot = TransportSlot()
    openPlaying(slot).use { media ->
      val first = media.suspendPlayback()
      val second = media.suspendPlayback()
      assertEquals("pause", withTimeout(10_000) { slot.log.receive() })

      first.release()
      first.release()
      media.setRate(CLIP, 2f)
      assertEquals("rate", withTimeout(10_000) { slot.log.receive() })

      second.close()
      assertEquals("play", withTimeout(10_000) { slot.log.receive() })
    }
  }

  @Test
  fun inv_session_12_a_group_every_suspension_allows_keeps_playing() = runBlocking {
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

  @Test
  fun inv_session_17_an_ungrouped_owner_is_in_the_default_group_under_a_suspension() = runBlocking {
    val slot = TransportSlot()
    openPlaying(slot, listOf(null, "system")).use { media ->
      val allowing = media.suspendPlayback(listOf("ring"))
      media.setRate(CLIP, 2f)
      assertEquals("rate", withTimeout(10_000) { slot.log.receive() })

      val blocking = media.suspendPlayback(listOf("system"))
      assertEquals("pause", withTimeout(10_000) { slot.log.receive() })
      blocking.release()
      assertEquals("play", withTimeout(10_000) { slot.log.receive() })
      allowing.release()
    }
  }

  @Test
  fun inv_session_17_the_default_group_leaves_the_set_when_the_ungrouped_owner_leaves() = runBlocking {
    val slot = TransportSlot()
    openPlaying(slot, listOf(null, "system")).use { media ->
      val suspension = media.suspendPlayback(listOf("ring"))
      media.setRate(CLIP, 2f)
      assertEquals("rate", withTimeout(10_000) { slot.log.receive() })

      media.setPlaying(CLIP, true, listOf("system"))
      assertEquals("pause", withTimeout(10_000) { slot.log.receive() })
      suspension.release()
    }
  }

  @Test
  fun inv_session_13_a_blank_allow_group_throws() {
    open(null).use { media ->
      assertThrows(MediaException::class.java) { media.suspendPlayback(listOf(" ")) }
    }
  }
}
