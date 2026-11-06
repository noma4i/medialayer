package com.noma4i.medialayer

import com.noma4i.medialayer.testing.TestFetcher
import java.nio.file.Files
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

private class CallSlot : PlayerSlot, PlayerSlotFactory {
  val log = Channel<String>(Channel.UNLIMITED)
  override suspend fun create(slot: UInt): CreatedPlayer = CreatedPlayer(this)
  override suspend fun apply(command: SlotCall) {
    when (command) {
      SlotCall.Play -> log.trySend("play")
      SlotCall.Pause -> log.trySend("pause")
      else -> {}
    }
  }
  override suspend fun dispose() {}
  override fun sample() = SlotSample(0u, null, false, false, false)
}

private class CallSettings : SettingsStore {
  override fun read(key: String): String? = null
  override fun write(key: String, value: String) {}
  override fun remove(key: String) {}
}

private const val CLIP = "file:///clip.mp4"
private val ITEM = MediaItem(Kind.VIDEO, CLIP, emptyMap(), null, null, strategy = CacheStrategy.WINDOW)

private suspend fun openPlaying(slot: CallSlot): MediaLayer {
  val media = MediaLayer.open(OpenConfig(availableBytes = ULong.MAX_VALUE,
    dir = Files.createTempDirectory("call").toString(),
    fetcher = TestFetcher(), settings = CallSettings(), slots = slot,
    device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings()
  ))
  media.replaceWindow(0u, listOf(ITEM))
  media.setPlaying(CLIP, true, listOf(null))
  assertEquals("play", withTimeout(10_000) { slot.log.receive() })
  return media
}

class CallHoldTest {
  init { NativeCallbacks.install() }

  @Test
  fun inv_call_12_aCallHoldPausesThePlaybackAndItsReleaseDoesNotResumeIt() = runBlocking {
    val slot = CallSlot()
    openPlaying(slot).use { media ->
      val hold = media.holdCall()
      assertEquals("pause", withTimeout(10_000) { slot.log.receive() })
      assertThrows(MediaException.InCall::class.java) { runBlocking { media.playUrl(ITEM) } }

      hold.release()
      media.playUrl(ITEM)
      assertTrue("the release resumed the playback", slot.log.tryReceive().isFailure)

      media.setPlaying(CLIP, true, listOf(null))
      assertEquals("play", withTimeout(10_000) { slot.log.receive() })
    }
  }
}
