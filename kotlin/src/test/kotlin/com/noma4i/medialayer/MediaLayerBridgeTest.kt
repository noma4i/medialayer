package com.noma4i.medialayer

import com.noma4i.medialayer.CreatedPlayer
import com.noma4i.medialayer.SlotCall
import java.nio.file.Files
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withTimeout
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withContext
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
  override fun failed(error: MediaException) = Unit
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

private class MemoryBody(data: ByteArray) : BodyReader {
  private var rest = data

  override suspend fun nextChunk(): ByteArray? {
    if (rest.isEmpty()) return null
    val count = minOf(rest.size, 4096)
    return rest.copyOfRange(0, count).also { rest = rest.copyOfRange(count, rest.size) }
  }

  override fun release() {}
}

private class RangeIgnoringOrigin : Fetcher {
  override suspend fun fetch(request: FetchRequest): FetchResponse = FetchResponse(200u, mapOf("Content-Type" to "audio/mp4"), MemoryBody(PAYLOAD))

  companion object {
    val PAYLOAD = ByteArray(20_000) { (it % 251).toByte() }
  }
}

private data class Poster(override val url: String) : MediaSource {
  override val kind: Kind get() = Kind.IMAGE
  override val headers: Map<String, String> get() = emptyMap()
}

private class LoggingSlot(val id: UInt) : PlayerSlot {
  val log = CopyOnWriteArrayList<String>()
  @Volatile var key: String? = null
  @Volatile var route: PlaybackRoute? = null

  @Volatile var generation: ULong = 0uL

  override suspend fun apply(command: SlotCall) {
    when (command) {
      is SlotCall.Load -> { key = command.key; route = command.route; generation = command.generation; log.add("load") }
      SlotCall.Unload -> log.add("unload")
      SlotCall.Preroll -> log.add("preroll")
      SlotCall.Play -> log.add("play")
      SlotCall.Pause -> log.add("pause")
      SlotCall.Mute -> log.add("mute")
      SlotCall.Unmute -> log.add("unmute")
      is SlotCall.Readahead -> log.add("readahead ${command.ms}")
      is SlotCall.Attach -> log.add("attach")
      SlotCall.Detach -> log.add("detach")
      is SlotCall.Seek -> log.add("seek")
      is SlotCall.Rate -> log.add("rate:${command.rate}")
    }
  }
  override suspend fun dispose() {}
  override fun sample(): SlotSample = SlotSample(0uL, null, false, false, false)
}

private class LoggingSlots : PlayerSlotFactory {
  val slots = ConcurrentHashMap<UInt, LoggingSlot>()

  override suspend fun create(slot: UInt): CreatedPlayer = CreatedPlayer(LoggingSlot(slot).also { slots[slot] = it })

  fun loaded(): LoggingSlot? = slots.values.firstOrNull { "load" in it.log }
}

private class StateRecorder : StateListener {
  val states = CopyOnWriteArrayList<MediaState>()

  override fun changed(state: MediaState) { states.add(state) }
  override fun failed(error: MediaException) = Unit
}

private fun eventually(what: String, timeoutMs: Long = 5_000, condition: () -> Boolean) {
  val deadline = System.nanoTime() + timeoutMs * 1_000_000
  while (!condition()) {
    if (System.nanoTime() > deadline) throw AssertionError("never became true within $timeoutMs ms: $what")
    Thread.sleep(10)
  }
}

class MediaLayerBridgeTest {
  init { NativeCallbacks.install() }

  @Test
  fun inv_image_47_theImageSizeReachesTheCallerFromTheHeadOfAFile() = runBlocking {
    val media = openWith(OfflineFetcher(), LoggingSlots(), silent = true)
    val head = intArrayOf(0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, 0x49, 0x48, 0x44, 0x52, 0, 0, 0x02, 0x80, 0, 0, 0x01, 0xe0, 8, 6, 0, 0, 0)
    val file = Files.createTempFile("size", ".png").toFile().apply { writeBytes(head.map { it.toByte() }.toByteArray()); deleteOnExit() }

    val size = media.imageSize(Poster("file://" + file.absolutePath))

    assertEquals(ImageSize(640u, 480u), size)
  }

  @Test
  fun inv_native_06_ten_page_changes_reuse_slots_and_attach_once_per_page() = runBlocking {
    val slots = LoggingSlots()
    val media = openWith(OfflineFetcher(), slots, silent = true)
    val pages = (0..12).map { MediaItem(Kind.VIDEO, "file:///bundle/$it.mp4", emptyMap(), null, null, strategy = CacheStrategy.WINDOW) }
    try {
      for (page in 1..11) {
        if (page > 1) {
          media.detachView(pages[page - 1].url)
          media.setPlaying(pages[page - 1].url, false)
        }
        media.replaceWindow(page.toUInt(), pages)
        media.setPlaying(pages[page].url, true)
        eventually("active source loaded") { slots.slots.values.any { it.key == pages[page].url } }
        media.attachView(pages[page].url, page.toULong())
        eventually("attachment delivered") { slots.slots.values.sumOf { it.log.count { call -> call == "attach" } } >= page }
        assertEquals(1u, media.stats().slots.playing)
      }
      val calls = slots.slots.values.flatMap { it.log }
      println("video-walk create=${slots.slots.size} load=${calls.count { it == "load" }} attach=${calls.count { it == "attach" }} detach=${calls.count { it == "detach" }} play=${calls.count { it == "play" }} preroll=${calls.count { it == "preroll" }}")
      assertEquals(3, slots.slots.size)
      assertEquals(11, calls.count { it == "attach" })
      assertEquals(10, calls.count { it == "detach" })
      assertEquals(11, calls.count { it == "play" })
    } finally {
      media.close()
    }
  }

  private fun openWith(fetcher: Fetcher, slots: PlayerSlotFactory, silent: Boolean): MediaLayer =
    MediaLayer.open(
      OpenConfig(availableBytes = ULong.MAX_VALUE,
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
      OpenConfig(availableBytes = ULong.MAX_VALUE,
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

    media.replaceImageWindow("grid", 0, (0 until 4).map { listOf(Poster("https://o.test/p$it.jpg")) })

    eventually("2 distinct urls were fetched") { fetcher.urls.toSet().size >= 2 }
    Thread.sleep(100)
    assertEquals(setOf("https://o.test/p0.jpg", "https://o.test/p1.jpg"), fetcher.urls.toSet())
  }

  @Test
  fun aSilentLayerMutesEverySlotAndNeverUnmutesWhateverThePlacesAsk() {
    val slots = LoggingSlots()
    val media = openWith(OfflineFetcher(), slots, silent = true)
    val item = MediaItem(Kind.VIDEO, "https://o.test/a.mp4", emptyMap(), null, null, strategy = CacheStrategy.WINDOW)

    media.visible(item, true)
    eventually("a slot holds a loaded key") { slots.loaded() != null }
    media.setMutedAll(true)
    eventually("the slot received the mute of mute-all") { slots.loaded()!!.log.count { it == "mute" } >= 2 }
    media.setMutedAll(false)
    media.setMuted(item.url, false)

    eventually("the slot turned the unmute of mute-all into a mute") { slots.loaded()!!.log.count { it == "mute" } >= 3 }
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
    val item = MediaItem(Kind.VIDEO, "file:///bundle/intro.mp4", emptyMap(), "https://o.test/cover.jpg", null, looping = false, strategy = CacheStrategy.WINDOW)
    val states = StateRecorder()
    val subscription = media.subscribeState(item.url, states)

    media.setRate(item.url, 1.5f)
    media.replaceWindow(0u, listOf(item, item.copy(url = "file:///bundle/next.mp4")))
    media.visible(item, true)
    media.setPlaying(item.url, true)
    eventually("the slot received play") { slots.loaded()?.log?.contains("play") == true }
    val slot = slots.loaded()!!
    media.playerEvents().ready(slot.id, slot.generation)
    media.playerEvents().visualReady(slot.id, slot.generation)
    eventually("the place state reached Rendered") { states.states.lastOrNull() == MediaState.Rendered }
    media.playerEvents().ended(slot.id, slot.generation)

    eventually("the place state reached Ended") { states.states.lastOrNull() == MediaState.Ended }
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
      OpenConfig(availableBytes = ULong.MAX_VALUE,
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
    eventually("the stats listener received the first stats") { recorder.totals.isNotEmpty() }
    assertEquals(listOf(0uL), recorder.totals.toList())
    subscription.close()
  }

  @Test
  fun inv_limits_08_reset_user_data_drops_the_cached_files_and_the_saved_limits() = runBlocking {
    val settings = MemorySettings()
    val media = MediaLayer.open(
      OpenConfig(availableBytes = ULong.MAX_VALUE,
        dir = Files.createTempDirectory("medialayer").toString(),
        fetcher = RangeIgnoringOrigin(),
        settings = settings,
        slots = null,
        device = DeviceMemory(totalMb = 4096u, heapClassMb = null, lowRam = false),
        defaults = defaultLimits(),
        pool = defaultPoolSettings()
      )
    )
    media.setLimits(media.limits().copy(budget = 512uL * 1024uL * 1024uL))
    media.fetchImage("https://o.test/user.jpg", emptyMap())
    assertEquals(1uL, media.stats().fileCount)
    assertEquals(setOf("medialayer.limits"), settings.values.keys)

    media.resetUserData()

    assertEquals(0uL, media.stats().fileCount)
    assertTrue(settings.values.isEmpty())
    assertEquals(defaultLimits(), media.limits())
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
  fun inv_prefetch_16_theChunkPolicyAndTheByteBudgetCrossTheBridgeAndAWrongListIsRefusedWithItsKey() = runBlocking {
    val limits = defaultLimits()
    assertEquals(listOf(2u, 1u), limits.leadingChunks)
    assertEquals(64u, limits.prefetchBudgetMib)
    val parsed = specDefaults("""{"cache":{"prefetch_budget_mib":96},"videos":[{"name":"clip","prefetch":{"from":1,"to":4,"chunks":[2,2]}}]}""")
    assertEquals(listOf(2u, 2u), parsed.defaults.leadingChunks)
    assertEquals(96u, parsed.defaults.prefetchBudgetMib)
    try {
      specDefaults("""{"videos":[{"name":"clip","prefetch":{"chunks":[1,2]}}]}""")
      fail("expected MediaException.InvalidSpec")
    } catch (error: MediaException.InvalidSpec) {
      assertTrue(error.detail.contains("leading_chunks"))
    }

    val media = open(MemorySettings())
    try {
      media.setLimits(defaultLimits().copy(leadingChunks = listOf(3u, 9u)))
      fail("expected MediaException.InvalidLimits")
    } catch (error: MediaException.InvalidLimits) {
      assertTrue(error.detail.contains("leading_chunks"))
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
    val source = MediaItem(Kind.VIDEO, "https://o.test/a.mp4", emptyMap(), null, null, strategy = CacheStrategy.WINDOW)

    val route = media.playUrl(source)

    assertTrue(route.url.startsWith("http://127.0.0.1:"))
    assertEquals(Placeholder.FirstFrame, placeholderFor(source, CacheState.MISSING, false))
  }

  @Test
  fun aPlatformHttpClientReadsTheFetcherBytesThroughTheLoopbackProxy() = runBlocking {
    openWith(RangeIgnoringOrigin(), LoggingSlots(), silent = true).use { media ->
      val route = media.playUrl(MediaItem(Kind.AUDIO, "https://o.test/voice.m4a", emptyMap(), null, null, strategy = CacheStrategy.WINDOW))
      val connection = java.net.URL(route.url).openConnection() as java.net.HttpURLConnection
      connection.setRequestProperty("Range", "bytes=0-1")

      val body = withContext(Dispatchers.IO) {
        System.gc()
        connection.inputStream.readBytes()
      }

      assertEquals(206, connection.responseCode)
      assertEquals("bytes 0-1/20000", connection.getHeaderField("Content-Range"))
      assertTrue(RangeIgnoringOrigin.PAYLOAD.copyOfRange(0, 2).contentEquals(body))
    }
  }
}
