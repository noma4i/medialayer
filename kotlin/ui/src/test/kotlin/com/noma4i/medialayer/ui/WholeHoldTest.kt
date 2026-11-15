package com.noma4i.medialayer.ui

import com.noma4i.medialayer.*
import java.nio.file.Files
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeoutOrNull
import org.junit.Assert.assertEquals
import org.junit.Assert.fail
import org.junit.Test
import org.junit.runner.RunWith

private class HeldOrigin : Fetcher {
  val fetched = java.util.Collections.synchronizedList(mutableListOf<String>())
  val released = AtomicInteger()
  val reading = java.util.Collections.synchronizedSet(mutableSetOf<String>())
  val changes = Channel<Unit>(Channel.UNLIMITED)

  fun fetches(name: String) = synchronized(fetched) { fetched.count { it.contains(name) } }

  fun counters() = "a=${fetches("a.jpg")} b=${fetches("b.jpg")} c=${fetches("c.jpg")} released=${released.get()}"

  override suspend fun fetch(request: FetchRequest): FetchResponse {
    fetched.add(request.url)
    changes.trySend(Unit)
    val body = object : BodyReader {
      override suspend fun nextChunk(): ByteArray? {
        reading.add(request.url)
        changes.trySend(Unit)
        awaitCancellation()
      }
      override fun release() { released.incrementAndGet(); changes.trySend(Unit) }
    }
    return FetchResponse(200u, emptyMap(), body)
  }
}

private suspend fun HeldOrigin.reaches(condition: () -> Boolean) {
  if (withTimeoutOrNull(10_000) { while (!condition()) changes.receive(); true } == null) fail("never reached: ${counters()}")
}

private fun photo(name: String) = MediaItem(Kind.IMAGE, "https://o.test/$name", emptyMap(), null, null, strategy = CacheStrategy.WHOLE)

@RunWith(MediaLayerTestRunner::class)
class WholeHoldTest {
  @Test
  fun inv_strategy_21_a_hold_opens_adds_and_closes_the_declaration() = runBlocking {
    val origin = HeldOrigin()
    val media = MediaLayer.open(OpenConfig(
      dir = Files.createTempDirectory("whole-hold").toString(),
      fetcher = origin,
      settings = object : SettingsStore {
        override fun read(key: String): String? = null
        override fun write(key: String, value: String) {}
        override fun remove(key: String) {}
      },
      slots = null, device = DeviceMemory(4096u, null, false), defaults = defaultLimits().copy(maxConcurrent = 3u), pool = defaultPoolSettings()
    ))
    val hold = MediaWholeHold()
    val first = photo("a.jpg")
    val second = photo("b.jpg")
    try {
      hold.declare(media, listOf(first))
      origin.reaches { origin.reading.any { it.contains("a.jpg") } }

      hold.declare(media, listOf(first, second))
      hold.declare(media, listOf(second))
      origin.reaches { origin.released.get() >= 1 && origin.fetches("b.jpg") >= 1 }
      assertEquals(origin.counters(), 1, origin.fetches("a.jpg"))

      hold.add(media, listOf(photo("c.jpg")))
      origin.reaches { origin.reading.any { it.contains("c.jpg") } }
      assertEquals(origin.counters(), 1, origin.released.get())
      assertEquals(origin.counters(), 1, origin.fetches("b.jpg"))

      hold.release()
      origin.reaches { origin.released.get() >= 2 }
    } finally {
      hold.release()
      media.close()
    }
  }
}
