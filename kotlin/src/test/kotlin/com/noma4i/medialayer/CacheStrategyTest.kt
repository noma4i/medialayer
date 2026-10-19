package com.noma4i.medialayer

import java.nio.file.Files
import java.util.concurrent.CopyOnWriteArrayList
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

private data class Request(val url: String, val start: ULong?, val end: ULong?)

private class StrategySettings : SettingsStore {
  override fun read(key: String): String? = null
  override fun write(key: String, value: String) {}
  override fun remove(key: String) {}
}

private class StrategyBody(private var rest: ByteArray) : BodyReader {
  override suspend fun nextChunk(): ByteArray? {
    if (rest.isEmpty()) return null
    val count = minOf(rest.size, 65_536)
    return rest.copyOfRange(0, count).also { rest = rest.copyOfRange(count, rest.size) }
  }

  override fun release() {}
}

private class RangeOrigin(private val refusal: (Request, Int) -> Boolean = { _, _ -> false }) : Fetcher {
  val requests = CopyOnWriteArrayList<Request>()

  override suspend fun fetch(request: FetchRequest): FetchResponse {
    val asked = Request(request.url, request.rangeStart, request.rangeEnd)
    val attempt = synchronized(requests) { requests.add(asked); requests.count { it == asked } }
    if (refusal(asked, attempt)) throw MediaException.Fetch("cut")
    val start = (request.rangeStart ?: 0uL).toInt()
    val end = minOf((request.rangeEnd ?: (TOTAL - 1).toULong()).toInt(), TOTAL - 1)
    val length = end - start + 1
    val headers = mutableMapOf("Content-Type" to "video/mp4", "Content-Length" to length.toString())
    if (request.rangeStart != null) headers["Content-Range"] = "bytes $start-$end/$TOTAL"
    return FetchResponse((if (request.rangeStart == null) 200 else 206).toUShort(), headers, StrategyBody(ByteArray(length) { 7 }))
  }

  companion object {
    const val TOTAL = 2 * 1024 * 1024 + 512 * 1024
    val BLOCKS = listOf(0uL to 1_048_575uL, 1_048_576uL to 2_097_151uL, 2_097_152uL to 2_621_439uL)
      .map { (start, end) -> Request("https://o.test/clip.mp4", start, end) }
  }
}

private data class GeneratedClip(override val url: String) : MediaSource {
  override val kind: Kind get() = Kind.VIDEO
  override val headers: Map<String, String> get() = emptyMap()
  override val strategy: CacheStrategy get() = CacheStrategy.WHOLE
}

private data class PlainPoster(override val url: String) : MediaSource {
  override val kind: Kind get() = Kind.IMAGE
  override val headers: Map<String, String> get() = emptyMap()
}

private fun item(url: String, kind: Kind, strategy: CacheStrategy) =
  MediaItem(kind, url, emptyMap(), null, null, strategy = strategy)

private fun eventuallyStrategy(what: String, timeoutMs: Long = 8_000, condition: () -> Boolean) {
  val deadline = System.nanoTime() + timeoutMs * 1_000_000
  while (!condition()) {
    if (System.nanoTime() > deadline) throw AssertionError("never became true within $timeoutMs ms: $what")
    Thread.sleep(10)
  }
}

class CacheStrategyTest {
  private fun open(fetcher: Fetcher, directory: String = Files.createTempDirectory("medialayer").toString()): MediaLayer =
    MediaLayer.open(
      OpenConfig(
        dir = directory,
        fetcher = fetcher,
        settings = StrategySettings(),
        slots = null,
        device = DeviceMemory(totalMb = 4096u, heapClassMb = null, lowRam = false),
        defaults = defaultLimits(),
        pool = defaultPoolSettings()
      )
    )

  @Test
  fun inv_strategy_15_a_whole_source_downloads_every_block_after_it_is_declared_and_hidden() {
    val origin = RangeOrigin()
    val media = open(origin)
    val clip = item("https://o.test/clip.mp4", Kind.VIDEO, CacheStrategy.WHOLE)

    media.visible(clip, true)
    media.visible(clip, false)

    eventuallyStrategy("3 blocks were requested") { origin.requests.size >= 3 }
    assertEquals(RangeOrigin.BLOCKS, origin.requests.toList())
  }

  @Test
  fun inv_strategy_15_a_whole_source_far_from_the_window_downloads_in_full() {
    val origin = RangeOrigin()
    val media = open(origin)
    val list: MutableList<MediaItem?> = (0 until 40).map { item("https://o.test/other$it.mp4", Kind.VIDEO, CacheStrategy.WINDOW) }.toMutableList()
    list[35] = item("https://o.test/clip.mp4", Kind.VIDEO, CacheStrategy.WHOLE)

    media.replaceWindow(0u, list)

    eventuallyStrategy("3 blocks of the far source were requested") { origin.requests.count { it.url == "https://o.test/clip.mp4" } >= 3 }
    assertEquals(RangeOrigin.BLOCKS, origin.requests.filter { it.url == "https://o.test/clip.mp4" })
  }

  @Test
  fun inv_strategy_15_the_image_window_skips_on_demand_sources() {
    val origin = RangeOrigin()
    val media = open(origin)

    media.replaceImageWindow("feed", 0u, (0 until 6).map { listOf(item("https://o.test/p$it.jpg", Kind.IMAGE, CacheStrategy.ON_DEMAND)) })
    Thread.sleep(400)

    assertTrue(origin.requests.isEmpty())
  }

  @Test
  fun inv_strategy_15_a_local_whole_source_is_never_fetched() {
    val origin = RangeOrigin()
    val media = open(origin)
    val local = item("file:///bundle/intro.mp4", Kind.VIDEO, CacheStrategy.WHOLE)

    media.visible(local, true)
    media.replaceWindow(0u, listOf(local))
    Thread.sleep(400)

    assertTrue(origin.requests.isEmpty())
  }

  @Test
  fun inv_strategy_15_a_failed_block_is_retried_and_the_download_completes() {
    val origin = RangeOrigin { request, attempt -> request.start == 1_048_576uL && attempt <= 2 }
    val media = open(origin)

    media.visible(item("https://o.test/clip.mp4", Kind.VIDEO, CacheStrategy.WHOLE), true)

    eventuallyStrategy("block 2 was requested", 15_000) { origin.requests.count { it.start == 2_097_152uL } == 1 }
    assertEquals(3, origin.requests.count { it.start == 1_048_576uL })
    assertEquals(1, origin.requests.count { it.start == 0uL })
  }

  @Test
  fun inv_strategy_15_a_download_cut_by_a_restart_resumes_at_the_first_missing_block() {
    val directory = Files.createTempDirectory("medialayer").toString()
    val cut = RangeOrigin { request, _ -> (request.start ?: 0uL) >= 2_097_152uL }
    val first = open(cut, directory)
    first.visible(item("https://o.test/clip.mp4", Kind.VIDEO, CacheStrategy.WHOLE), true)
    eventuallyStrategy("block 2 failed 3 times", 15_000) { cut.requests.count { it.start == 2_097_152uL } == 3 }
    Thread.sleep(2_000)
    first.close()

    val healthy = RangeOrigin()
    open(healthy, directory).visible(item("https://o.test/clip.mp4", Kind.VIDEO, CacheStrategy.WHOLE), true)

    eventuallyStrategy("the missing block was requested") { healthy.requests.isNotEmpty() }
    Thread.sleep(300)
    assertEquals(listOf(RangeOrigin.BLOCKS[2]), healthy.requests.toList())
  }

  @Test
  fun inv_strategy_15_a_generated_source_declares_its_strategy_and_the_default_is_the_window() {
    assertEquals(CacheStrategy.WHOLE, GeneratedClip("https://o.test/clip.mp4").toItem().strategy)
    assertEquals(CacheStrategy.WINDOW, PlainPoster("https://o.test/p.jpg").toItem().strategy)
  }
}
