package com.noma4i.medialayer.ui

import com.noma4i.medialayer.BodyReader
import com.noma4i.medialayer.FetchRequest
import com.noma4i.medialayer.MediaException
import java.io.ByteArrayOutputStream
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

class OkHttpFetcherTest {
  private suspend fun BodyReader.readAll(): String {
    val out = ByteArrayOutputStream()
    while (true) out.write(nextChunk() ?: break)
    return out.toString(Charsets.UTF_8)
  }

  private suspend fun BodyReader.readAtLeast(count: Int): String {
    val out = ByteArrayOutputStream()
    while (out.size() < count) out.write(nextChunk() ?: break)
    return out.toString(Charsets.UTF_8)
  }

  @Test
  fun requestCarriesHeadersAndRangeAndTheResponseKeepsStatusHeadersAndBody() = runBlocking {
    val ranges = listOf(Triple(10uL, 19uL, "bytes=10-19"), Triple(10uL, null, "bytes=10-"), Triple(null, null, null))
    for ((start, end, expected) in ranges) {
      TestServer { it.respond("206 Partial Content", "0123456789", "X-Origin: cdn\r\nConnection: close\r\n") }.use { server ->
        val response = OkHttpFetcher().fetch(FetchRequest(server.url, mapOf("Authorization" to "token"), start, end))
        val head = server.heads.take()

        assertEquals(expected, head.headers["range"])
        assertEquals("token", head.headers["authorization"])
        assertEquals(206.toUShort(), response.status)
        assertEquals("cdn", response.headers["x-origin"])
        assertEquals("0123456789", response.body.readAll())
      }
    }
  }

  @Test
  fun bodyArrivesChunkByChunkWhileTheServerStillHoldsTheRest() = runBlocking {
    val release = CountDownLatch(1)
    TestServer { socket ->
      socket.getOutputStream().apply { write("HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nAAAA".toByteArray()); flush() }
      release.await(10, TimeUnit.SECONDS)
      socket.getOutputStream().apply { write("BBBB".toByteArray()); flush() }
    }.use { server ->
      val body = OkHttpFetcher().fetch(FetchRequest(server.url, emptyMap(), null, null)).body

      assertEquals("AAAA", withTimeout(10_000) { body.readAtLeast(4) })
      release.countDown()
      assertEquals("BBBB", withTimeout(10_000) { body.readAtLeast(4) })
      assertNull(body.nextChunk())
    }
  }

  @Test
  fun cancellingABlockedBodyReadClosesTheConnection() = runBlocking {
    val gone = CountDownLatch(1)
    TestServer { socket ->
      socket.getOutputStream().apply { write("HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nAAAA".toByteArray()); flush() }
      socket.waitUntilClientLeaves()
      gone.countDown()
    }.use { server ->
      val body = OkHttpFetcher().fetch(FetchRequest(server.url, emptyMap(), null, null)).body
      body.readAtLeast(4)

      val pending = async(start = CoroutineStart.UNDISPATCHED) { body.nextChunk() }
      pending.cancel()

      val failure = runCatching { pending.await() }.exceptionOrNull()
      assertTrue(failure.toString(), failure is CancellationException)
      assertTrue(gone.await(10, TimeUnit.SECONDS))
    }
  }

  @Test
  fun cancellingAFetchWaitingForHeadersClosesTheConnection() = runBlocking {
    val gone = CountDownLatch(1)
    TestServer { socket -> socket.waitUntilClientLeaves(); gone.countDown() }.use { server ->
      val pending = async(Dispatchers.IO) { OkHttpFetcher().fetch(FetchRequest(server.url, emptyMap(), null, null)) }
      server.heads.take()
      pending.cancel()

      assertTrue(gone.await(10, TimeUnit.SECONDS))
    }
  }

  @Test
  fun releasingTheBodyClosesTheConnection() = runBlocking {
    val gone = CountDownLatch(1)
    TestServer { socket ->
      socket.getOutputStream().apply { write("HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nAAAA".toByteArray()); flush() }
      socket.waitUntilClientLeaves()
      gone.countDown()
    }.use { server ->
      val body = OkHttpFetcher().fetch(FetchRequest(server.url, emptyMap(), null, null)).body
      body.readAtLeast(4)

      body.release()

      assertTrue(gone.await(10, TimeUnit.SECONDS))
    }
  }

  @Test
  fun aSilentServerEndsWithAFetchExceptionAfterTheTimeout() = runBlocking {
    TestServer { socket -> socket.waitUntilClientLeaves() }.use { server ->
      try {
        withTimeout(5_000) { OkHttpFetcher(timeoutMs = 200).fetch(FetchRequest(server.url, emptyMap(), null, null)) }
        fail("expected MediaException.Fetch")
      } catch (error: MediaException.Fetch) {
        assertTrue(error.detail.isNotEmpty())
      }
    }
  }
}
