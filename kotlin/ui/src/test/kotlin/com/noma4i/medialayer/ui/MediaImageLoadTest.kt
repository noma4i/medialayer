package com.noma4i.medialayer.ui

import com.noma4i.medialayer.ImageOrigin
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.testing.TestFetcher
import java.io.File
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(MediaLayerTestRunner::class)
class MediaImageLoadTest {
  private val address = "https://img.example/avatar.png"
  private val payload = ByteArray(300) { 7 }

  private fun directory(): File = java.nio.file.Files.createTempDirectory("image-load").toFile()

  @Test
  fun inv_image_56_second_load_of_a_url_comes_from_the_cache_without_network() = runBlocking {
    val fetcher = TestFetcher().apply { serve(address, payload) }
    val directory = directory()

    val first = MediaImageLoad.load(address, directory, fetcher = fetcher)
    val second = MediaImageLoad.load(address, directory, fetcher = fetcher)

    assertEquals(ImageOrigin.NETWORK, first.source)
    assertEquals(ImageOrigin.CACHE, second.source)
    assertArrayEquals(payload, second.bytes)
    assertEquals(1, fetcher.requestCount(address))
  }

  @Test
  fun inv_image_58_slow_origin_ends_with_a_timeout_error() = runBlocking {
    val fetcher = TestFetcher().apply { serve(address, payload); delay(address, 30_000) }
    try {
      MediaImageLoad.load(address, directory(), timeoutMs = 200, fetcher = fetcher)
      fail("expected a timeout")
    } catch (error: MediaException.Timeout) {
      assertTrue(error.message.orEmpty().isNotEmpty())
    }
  }

  @Test
  fun inv_android_19_load_does_not_block_the_calling_thread() = runBlocking {
    val fetcher = TestFetcher().apply { serve(address, payload); delay(address, 400) }
    val ticks = AtomicInteger()
    val heartbeat = launch { while (isActive) { ticks.incrementAndGet(); delay(10) } }

    MediaImageLoad.load(address, directory(), fetcher = fetcher)
    heartbeat.cancel()

    assertTrue("ticks=${ticks.get()}", ticks.get() >= 10)
  }
}
