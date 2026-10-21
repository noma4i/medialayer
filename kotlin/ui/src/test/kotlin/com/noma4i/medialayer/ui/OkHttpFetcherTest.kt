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
import kotlin.coroutines.CoroutineContext
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.channels.Channel
import okhttp3.OkHttpClient
import okhttp3.Protocol
import okhttp3.Response
import okhttp3.ResponseBody
import okio.Buffer
import okio.ForwardingSource
import okio.buffer
import java.util.concurrent.atomic.AtomicBoolean

class OkHttpFetcherTest {
  @Test
  fun inv_proxy_07_truncated_bodies_report_fetch_failure_instead_of_eof() = runBlocking {
    TestServer { socket ->
      socket.getOutputStream().apply {
        write("HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nshort".toByteArray())
        flush()
      }
    }.use { server ->
      val body = OkHttpFetcher().fetch(FetchRequest(server.url, emptyMap(), null, null)).body
      try {
        val failure = runCatching { body.readAll() }.exceptionOrNull()
        assertTrue(failure.toString(), failure is MediaException.Fetch)
      } finally { body.release() }
    }
  }

  @Test
  fun inv_proxy_06_full_chunks_transfer_the_read_array_and_short_chunks_keep_their_length() = runBlocking {
    val inputs = mutableListOf<ByteArray>()
    val bytes = Buffer().write(ByteArray(64 * 1024) { 1 }).write(ByteArray(7) { 2 })
    val source = java.lang.reflect.Proxy.newProxyInstance(
      okio.BufferedSource::class.java.classLoader, arrayOf(okio.BufferedSource::class.java)
    ) { _, method, args ->
      if (method.name == "read" && args?.firstOrNull() is ByteArray) {
        val target = args[0] as ByteArray
        inputs.add(target)
        val count = minOf(target.size.toLong(), bytes.size).toInt()
        if (count == 0) -1 else {
          var offset = 0
          while (offset < count) offset += bytes.read(target, offset, count - offset)
          count
        }
      } else method.invoke(bytes, *(args ?: emptyArray()))
    } as okio.BufferedSource
    val responseBody = object : ResponseBody() {
      override fun contentType() = null
      override fun contentLength() = 65543L
      override fun source() = source
    }
    val client = OkHttpClient.Builder().addInterceptor { chain ->
      Response.Builder().request(chain.request()).protocol(Protocol.HTTP_1_1).code(200).message("OK").body(responseBody).build()
    }.build()
    val body = OkHttpFetcher(client).fetch(FetchRequest("https://o.test/body", emptyMap(), null, null)).body
    val full = body.nextChunk()!!
    val short = body.nextChunk()!!
    assertNull(body.nextChunk())
    assertEquals(65536, full.size)
    assertEquals(7, short.size)
    assertTrue(full.all { it == 1.toByte() })
    assertTrue(short.all { it == 2.toByte() })
    println("full_chunk_extra_arrays=${if (inputs[0] === full) 0 else 1}, reads=${inputs.size}")
    org.junit.Assert.assertSame(inputs[0], full)
    org.junit.Assert.assertNotSame(inputs[1], short)
    body.release()
  }

  @Test
  fun cancellingAfterHeadersBeforeDeliveryClosesTheUndeliveredResponse() = runBlocking {
    val queued = Channel<Runnable>(Channel.UNLIMITED)
    val dispatcher = object : CoroutineDispatcher() {
      override fun dispatch(context: CoroutineContext, block: Runnable) { queued.trySend(block) }
    }
    val closed = AtomicBoolean()
    val body = object : ResponseBody() {
      private val bytes = object : ForwardingSource(Buffer().writeUtf8("body")) {
        override fun close() { closed.set(true); super.close() }
      }.buffer()
      override fun contentType() = null
      override fun contentLength() = 4L
      override fun source() = bytes
    }
    val client = OkHttpClient.Builder().addInterceptor { chain ->
      Response.Builder().request(chain.request()).protocol(Protocol.HTTP_1_1).code(200).message("OK").body(body).build()
    }.build()
    val pending = async(dispatcher, start = CoroutineStart.UNDISPATCHED) {
      OkHttpFetcher(client).fetch(FetchRequest("https://o.test/body", emptyMap(), null, null))
    }
    val delivered = withTimeout(10_000) { queued.receive() }
    pending.cancel()
    delivered.run()
    pending.join()
    assertTrue(closed.get())
  }

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
