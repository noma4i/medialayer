package com.noma4i.medialayer

import java.nio.file.Files
import java.util.concurrent.CopyOnWriteArrayList
import org.junit.Assert.assertEquals
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

private class RangeOrigin : Fetcher {
  val requests = CopyOnWriteArrayList<Request>()

  override suspend fun fetch(request: FetchRequest): FetchResponse {
    requests.add(Request(request.url, request.rangeStart, request.rangeEnd))
    TestSignal.fire()
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

private fun eventuallyStrategy(origin: RangeOrigin, what: String, timeoutMs: Long = 8_000, condition: () -> Boolean) {
  TestSignal.await(what, timeoutMs, condition)
}

class CacheStrategyTest {
  init { NativeCallbacks.install() }

  @Test
  fun inv_strategy_15_a_whole_source_downloads_every_block_after_it_is_declared_and_hidden() {
    val origin = RangeOrigin()
    val media = MediaLayer.open(
      OpenConfig(
        dir = Files.createTempDirectory("medialayer").toString(),
        fetcher = origin,
        settings = StrategySettings(),
        slots = null,
        device = DeviceMemory(totalMb = 4096u, heapClassMb = null, lowRam = false),
        defaults = defaultLimits(),
        pool = defaultPoolSettings()
      )
    )
    val clip = MediaItem(Kind.VIDEO, "https://o.test/clip.mp4", emptyMap(), null, null, strategy = CacheStrategy.WHOLE)
    try {
      media.visible(clip, true)
      eventuallyStrategy(origin, "the first block was requested") { origin.requests.isNotEmpty() }
      media.visible(clip, false)

      eventuallyStrategy(origin, "3 blocks were requested") { origin.requests.size >= RangeOrigin.BLOCKS.size }
      assertEquals(RangeOrigin.BLOCKS, origin.requests.toList())
    } finally {
      media.close()
    }
  }
}
