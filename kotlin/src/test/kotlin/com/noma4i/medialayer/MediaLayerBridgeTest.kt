package com.noma4i.medialayer

import java.nio.file.Files
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicInteger
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

private class CountingFetcher : Fetcher {
  val count = AtomicInteger()

  override suspend fun fetch(request: FetchRequest): FetchResponse {
    count.incrementAndGet()
    throw MediaException.Fetch("offline")
  }
}

private class UrlFetcher : Fetcher {
  val urls = CopyOnWriteArrayList<String>()

  override suspend fun fetch(request: FetchRequest): FetchResponse {
    urls.add(request.url)
    throw MediaException.Fetch("offline")
  }
}

private data class Poster(override val url: String) : MediaSource {
  override val kind: Kind get() = Kind.IMAGE
  override val headers: Map<String, String> get() = emptyMap()
}

private class LoggingSlot(val id: UInt) : PlayerSlot {
  val log = CopyOnWriteArrayList<String>()
  @Volatile var route: PlaybackRoute? = null

  override fun load(key: String, route: PlaybackRoute) { this.route = route; log.add("load") }
  override fun unload() { log.add("unload") }
  override fun preroll() { log.add("preroll") }
  override fun play() { log.add("play") }
  override fun pause() { log.add("pause") }
  override fun mute() { log.add("mute") }
  override fun unmute() { log.add("unmute") }
  override fun retain(level: RetainLevel) { log.add("retain") }
  override fun attach(viewToken: ULong) { log.add("attach") }
  override fun detach() { log.add("detach") }
  override fun seek(positionMs: ULong) { log.add("seek") }
  override fun setRate(rate: Float) { log.add("rate:$rate") }
  override fun sample(): SlotSample = SlotSample(0uL, null, false, false, false)
}

private class LoggingSlots : PlayerSlotFactory {
  val slots = ConcurrentHashMap<UInt, LoggingSlot>()

  override fun create(slot: UInt): PlayerSlot = LoggingSlot(slot).also { slots[slot] = it }

  fun loaded(): LoggingSlot? = slots.values.firstOrNull { "load" in it.log }
}

private class StateRecorder : StateListener {
  val states = CopyOnWriteArrayList<MediaState>()

  override fun changed(state: MediaState) { states.add(state) }
}

private fun eventually(condition: () -> Boolean) {
  repeat(500) { if (!condition()) Thread.sleep(10) }
  assertTrue("condition never became true", condition())
}

class MediaLayerBridgeTest {
  private fun openWith(fetcher: Fetcher, slots: PlayerSlotFactory, silent: Boolean): MediaLayer =
    MediaLayer.open(
      OpenConfig(
        dir = Files.createTempDirectory("medialayer").toString(),
        fetcher = fetcher,
        settings = MemorySettings(),
        slots = slots,
        device = DeviceMemory(totalMb = 4096u, heapClassMb = null, lowRam = false),
        defaults = defaultLimits(),
        pool = defaultPoolSettings(),
        silent = silent
      )
    )

  @Test
  fun inv_api_17_a_typed_image_window_prefetches_the_remote_images_of_its_pages() {
    val fetcher = UrlFetcher()
    val media = MediaLayer.open(
      OpenConfig(
        dir = Files.createTempDirectory("medialayer").toString(),
        fetcher = fetcher,
        settings = MemorySettings(),
        slots = null,
        device = DeviceMemory(totalMb = 4096u, heapClassMb = null, lowRam = false),
        defaults = defaultLimits(),
        pool = defaultPoolSettings(),
        imageWindows = listOf(ImageWindow("grid", 0u, 0u, 2u))
      )
    )

    media.setImageWindow("grid", 0, (0 until 4).map { listOf(Poster("https://o.test/p$it.jpg")) })

    eventually { fetcher.urls.toSet().size >= 2 }
    Thread.sleep(100)
    assertEquals(setOf("https://o.test/p0.jpg", "https://o.test/p1.jpg"), fetcher.urls.toSet())
  }

  @Test
  fun aSilentLayerMutesEverySlotAndNeverUnmutesWhateverThePlacesAsk() {
    val slots = LoggingSlots()
    val media = openWith(OfflineFetcher(), slots, silent = true)
    val item = MediaItem(Kind.VIDEO, "https://o.test/a.mp4", emptyMap(), null, null)

    media.visible(item, true)
    eventually { slots.loaded() != null }
    media.setMutedAll(true)
    media.setMutedAll(false)
    media.setMuted(item.url, false)

    eventually { slots.loaded()!!.log.count { it == "mute" } >= 2 }
    Thread.sleep(200)
    val log = slots.loaded()!!.log.toList()
    assertEquals("mute", log[log.indexOf("load") + 1])
    assertTrue("unmute" !in log)
  }

  @Test
  fun aLocalOneShotClipPlaysFromItsFileEndsAndNeverTouchesTheNetworkOrTheCache() = runBlocking {
    val slots = LoggingSlots()
    val fetcher = CountingFetcher()
    val media = openWith(fetcher, slots, silent = true)
    val item = MediaItem(Kind.VIDEO, "file:///bundle/intro.mp4", emptyMap(), "https://o.test/cover.jpg", null, looping = false)
    val states = StateRecorder()
    val subscription = media.subscribeState(item.url, states)

    media.setRate(item.url, 1.5f)
    media.setWindow(0u, listOf(item, item.copy(url = "file:///bundle/next.mp4")))
    media.visible(item, true)
    media.setPlaying(item.url, true)
    eventually { slots.loaded()?.log?.contains("play") == true }
    val slot = slots.loaded()!!
    media.playerEvents().ready(slot.id)
    media.playerEvents().visualReady(slot.id)
    eventually { states.states.lastOrNull() == MediaState.Rendered }
    media.playerEvents().ended(slot.id)

    eventually { states.states.lastOrNull() == MediaState.Ended }
    assertEquals(listOf<Any?>("file:///bundle/intro.mp4", false, null), listOf(slot.route!!.url, slot.route!!.looping, slot.route!!.generation))
    assertTrue("rate:1.5" in slot.log)
    Thread.sleep(200)
    assertEquals(0, fetcher.count.get())
    assertEquals(0uL, media.stats().fileCount)
    assertEquals(Placeholder.None, media.placeholder(item, false))
    subscription.close()
  }

  @Test
  fun specDefaultsCarryTheSpecLimitsPoolAndNamespaceHeadersAndRefuseABrokenSpec() {
    val parsed = specDefaults("""{"cache":{"budget":3000000000,"namespace_headers":["Authorization"]},"videos":[{"name":"clip","pool":{"slots":5}}]}""")

    assertEquals(listOf<Any>(3_000_000_000uL, 5u, listOf("Authorization")), listOf(parsed.defaults.budget, parsed.defaults.slots, parsed.namespaceHeaders))
    try {
      specDefaults("{")
      fail("expected MediaException.InvalidSpec")
    } catch (error: MediaException.InvalidSpec) {
      assertTrue(error.detail.isNotEmpty())
    }
  }

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
  fun playUrlGoesThroughTheLocalProxyAndTheVideoPlaceholderIsTheFirstFrame() = runBlocking {
    val media = open(MemorySettings())
    val source = MediaItem(Kind.VIDEO, "https://o.test/a.mp4", emptyMap(), null, null)

    val route = media.playUrl(source)

    assertTrue(route.url.startsWith("http://127.0.0.1:"))
    assertEquals(Placeholder.FirstFrame, placeholderFor(source, CacheState.MISSING, false))
  }
}
