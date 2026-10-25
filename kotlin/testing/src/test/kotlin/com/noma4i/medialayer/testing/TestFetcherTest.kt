package com.noma4i.medialayer.testing

import com.noma4i.medialayer.BodyReader
import com.noma4i.medialayer.FetchRequest
import com.noma4i.medialayer.MediaException
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeoutOrNull
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class TestFetcherTest {
  private val payload = ByteArray(10_000) { (it % 251).toByte() }
  private val url = "https://o.test/a.mp4"
  private val fetcher = TestFetcher().also { it.serve(url, payload, "video/mp4") }

  private fun request(start: ULong? = null, end: ULong? = null) = FetchRequest(url, emptyMap(), start, end)

  private fun BodyReader.readAll(): ByteArray = runBlocking {
    val out = java.io.ByteArrayOutputStream()
    while (true) out.write(nextChunk() ?: break)
    out.toByteArray()
  }

  @Test
  fun inv_native_32_testFetcherAnswersRangesLikeAnOrigin() = runBlocking {
    val whole = fetcher.fetch(request())
    assertEquals(200.toUShort(), whole.status)
    assertEquals("video/mp4", whole.headers["Content-Type"])
    assertArrayEquals(payload, whole.body.readAll())

    val middle = fetcher.fetch(request(10uL, 19uL))
    assertEquals(206.toUShort(), middle.status)
    assertEquals("bytes 10-19/10000", middle.headers["Content-Range"])
    assertArrayEquals(payload.copyOfRange(10, 20), middle.body.readAll())

    val tail = fetcher.fetch(request(9_990uL, null))
    assertEquals("bytes 9990-9999/10000", tail.headers["Content-Range"])
    assertArrayEquals(payload.copyOfRange(9_990, 10_000), tail.body.readAll())

    val clamped = fetcher.fetch(request(9_990uL, 50_000uL))
    assertEquals("bytes 9990-9999/10000", clamped.headers["Content-Range"])
    assertEquals(10, clamped.body.readAll().size)

    val beyond = fetcher.fetch(request(10_000uL, null))
    assertEquals(416.toUShort(), beyond.status)
    assertEquals("bytes */10000", beyond.headers["Content-Range"])
    assertEquals(0, beyond.body.readAll().size)
  }

  @Test
  fun inv_native_33_testFetcherCountsRequestsAndAppliesFailureAndDelayPerUrl() = runBlocking {
    fetcher.fail("https://o.test/bad", "boom")
    fetcher.serve("https://o.test/slow", payload)
    fetcher.delay("https://o.test/slow", 60_000)

    fetcher.fetch(request()); fetcher.fetch(request(0uL, 1uL))
    try { fetcher.fetch(FetchRequest("https://o.test/bad", emptyMap(), null, null)); fail() } catch (error: MediaException.Fetch) { assertTrue(error.message!!.contains("boom")) }
    try { fetcher.fetch(FetchRequest("https://o.test/unknown", emptyMap(), null, null)); fail() } catch (error: MediaException.Fetch) { assertTrue(error.message!!.contains("offline")) }
    assertNull(withTimeoutOrNull(50) { fetcher.fetch(FetchRequest("https://o.test/slow", emptyMap(), null, null)) })

    assertEquals(2, fetcher.requestCount(url))
    assertEquals(1, fetcher.requestCount("https://o.test/bad"))
    assertEquals(1, fetcher.requestCount("https://o.test/slow"))
    assertEquals(5, fetcher.requests.size)
  }
}
