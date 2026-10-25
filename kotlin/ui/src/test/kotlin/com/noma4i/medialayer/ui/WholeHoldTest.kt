package com.noma4i.medialayer.ui

import com.noma4i.medialayer.*
import java.nio.file.Files
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeoutOrNull
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

private class HeldOrigin : Fetcher {
  val fetched = java.util.Collections.synchronizedList(mutableListOf<String>())
  val released = AtomicInteger()

  fun fetches(name: String) = synchronized(fetched) { fetched.count { it.contains(name) } }

  override suspend fun fetch(request: FetchRequest): FetchResponse {
    fetched.add(request.url)
    val body = object : BodyReader {
      override suspend fun nextChunk(): ByteArray? = awaitCancellation()
      override fun release() { released.incrementAndGet() }
    }
    return FetchResponse(200u, emptyMap(), body)
  }
}

private fun photo(name: String) = MediaItem(Kind.IMAGE, "https://o.test/$name", emptyMap(), null, null, strategy = CacheStrategy.WHOLE)

private suspend fun waits(condition: () -> Boolean): Boolean = withTimeoutOrNull(10_000) {
  while (!condition()) delay(25)
  true
} ?: false

@RunWith(MediaLayerTestRunner::class)
class WholeHoldTest {
  @Test
  fun inv_strategy_21_a_hold_opens_adds_and_closes_the_declaration() = runBlocking {
    val origin = HeldOrigin()
    val media = MediaLayer.open(OpenConfig(availableBytes = ULong.MAX_VALUE,
      dir = Files.createTempDirectory("whole-hold").toString(),
      fetcher = origin,
      settings = object : SettingsStore {
        override fun read(key: String): String? = null
        override fun write(key: String, value: String) {}
        override fun remove(key: String) {}
      },
      slots = null, device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings()
    ))
    val hold = MediaWholeHold()
    val first = photo("a.jpg")
    val second = photo("b.jpg")
    try {
      hold.declare(media, listOf(first))
      assertTrue(waits { origin.fetches("a.jpg") == 1 })

      hold.declare(media, listOf(first, second))
      assertEquals(1, origin.fetches("a.jpg"))

      hold.declare(media, listOf(second))
      assertTrue(waits { origin.released.get() == 1 })
      assertTrue(waits { origin.fetches("b.jpg") == 1 })

      hold.add(media, listOf(photo("c.jpg")))
      assertEquals(1, origin.released.get())

      hold.release()
      assertTrue(waits { origin.released.get() >= 2 })
    } finally {
      hold.release()
      media.close()
    }
  }
}
