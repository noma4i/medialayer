package com.noma4i.medialayer

import java.nio.file.Files
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.async
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

private class MemorySettings : SettingsStore {
  val values = ConcurrentHashMap<String, String>()

  override fun read(key: String): String? = values[key]
  override fun write(key: String, value: String) { values[key] = value }
  override fun remove(key: String) { values.remove(key) }
}

private class OfflineFetcher : Fetcher {
  override suspend fun fetch(request: FetchRequest): FetchResponse = throw MediaException.Fetch("offline")
}

private class StatsRecorder : StatsListener {
  val totals = CopyOnWriteArrayList<ULong>()

  override fun changed(stats: MediaStats) { totals.add(stats.totalBytes) }
}

class MediaLayerBridgeTest {
  private fun open(settings: MemorySettings): MediaLayer =
    MediaLayer.open(
      OpenConfig(
        dir = Files.createTempDirectory("medialayer").toString(),
        fetcher = OfflineFetcher(),
        settings = settings,
        slots = null,
        device = DeviceMemory(totalMb = 4096u, heapClassMb = null, lowRam = false),
        defaults = defaultLimits(),
        pool = defaultPoolSettings()
      )
    )

  @Test
  fun limitsReachThePlatformSettingsAndStatsAnswer() = runBlocking {
    val settings = MemorySettings()
    val media = open(settings)
    val recorder = StatsRecorder()
    val subscription = media.subscribeStats(recorder)
    val limits = media.limits().copy(budget = 512uL * 1024uL * 1024uL, evict = Evict.Lfu)

    media.setLimits(limits)

    val stats = media.stats()
    assertEquals(limits, media.limits())
    assertEquals(setOf("medialayer.limits"), settings.values.keys)
    assertEquals(0uL, stats.totalBytes)
    assertEquals(0uL, stats.fileCount)
    repeat(200) { if (recorder.totals.isEmpty()) delay(10) }
    assertEquals(listOf(0uL), recorder.totals.toList())
    subscription.close()
  }

  @Test
  fun invalidLimitsThrowAMediaExceptionWithDetail() = runBlocking {
    val media = open(MemorySettings())

    try {
      media.setLimits(defaultLimits().copy(budget = 1uL))
      fail("expected MediaException.InvalidLimits")
    } catch (error: MediaException.InvalidLimits) {
      assertTrue(error.detail.isNotEmpty())
    }
  }

  @Test
  fun statsFlowDeliversTheCurrentStats() = runBlocking {
    val media = open(MemorySettings())
    val collected = async { withTimeout(5000) { media.statsFlow().first() } }

    delay(50)
    media.evictNow()

    assertEquals(0uL, collected.await().totalBytes)
  }

  @Test
  fun stateFlowStartsIdleForAnUndeclaredPlace() = runBlocking {
    val media = open(MemorySettings())

    val first = withTimeout(5000) { media.stateFlow("https://o.test/a.mp4").first() }

    assertEquals(MediaState.Idle, first)
  }

  @Test
  fun playUrlGoesThroughTheLocalProxyAndTheVideoPlaceholderIsTheFirstFrame() {
    val media = open(MemorySettings())
    val source = MediaItem(Kind.VIDEO, "https://o.test/a.mp4", emptyMap(), null, null)

    val route = media.playUrl(source)

    assertTrue(route.url.startsWith("http://127.0.0.1:"))
    assertEquals(Placeholder.FirstFrame, placeholderFor(source, CacheState.MISSING, false))
  }
}
