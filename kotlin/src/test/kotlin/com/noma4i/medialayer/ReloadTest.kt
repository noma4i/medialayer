package com.noma4i.medialayer

import java.nio.file.Files
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertTrue
import org.junit.Test

private class ReloadSlot : PlayerSlot, PlayerSlotFactory {
  val plays = Channel<Boolean>(Channel.UNLIMITED)
  @Volatile private var attached = false
  override fun create(slot: UInt): PlayerSlot = this
  override fun load(key: String, route: PlaybackRoute) {}
  override fun unload() { attached = false }
  override fun preroll() {}
  override fun play() { plays.trySend(attached) }
  override fun pause() {}
  override fun mute() {}
  override fun unmute() {}
  override fun attach(viewToken: ULong) { attached = viewToken == 42uL }
  override fun detach() { attached = false }
  override fun seek(positionMs: ULong) {}
  override fun setRate(rate: Float) {}
  override fun setReadahead(ms: ULong) {}
  override fun sample() = SlotSample(0u, null, false, attached, false)
}

private class ReloadSettings : SettingsStore {
  override fun read(key: String): String? = null
  override fun write(key: String, value: String) {}
  override fun remove(key: String) {}
}

private class ReloadFetcher : Fetcher {
  override suspend fun fetch(request: FetchRequest): FetchResponse = throw MediaException.Fetch("offline")
}

class ReloadTest {
  init { NativeCallbacks.install() }

  @Test
  fun inv_api_06_reload_restores_the_attached_view_before_playing() = runBlocking {
    val slot = ReloadSlot()
    val media = MediaLayer.open(OpenConfig(availableBytes = ULong.MAX_VALUE,
      dir = Files.createTempDirectory("reload").toString(),
      fetcher = ReloadFetcher(), settings = ReloadSettings(), slots = slot,
      device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings()
    ))
    val key = "file:///clip.mp4"
    val events = media.playerEvents()
    events.attach(key, 42u)
    media.replaceWindow(0u, listOf(MediaItem(Kind.VIDEO, key, emptyMap(), null, null, strategy = CacheStrategy.WINDOW)))
    media.setPlaying(key, true)
    withTimeout(10_000) {
      assertTrue(slot.plays.receive())
      events.failed(0u, 0, "decoder failed")
      assertTrue(slot.plays.receive())
    }
  }
}
