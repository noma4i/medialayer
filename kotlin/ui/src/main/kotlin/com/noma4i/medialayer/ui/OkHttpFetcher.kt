package com.noma4i.medialayer.ui

import com.noma4i.medialayer.BodyReader
import com.noma4i.medialayer.FetchRequest
import com.noma4i.medialayer.FetchResponse
import com.noma4i.medialayer.Fetcher
import com.noma4i.medialayer.MediaException
import java.io.IOException
import java.util.concurrent.TimeUnit
import kotlin.coroutines.resumeWithException
import kotlin.coroutines.resume
import kotlinx.coroutines.CancellableContinuation
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import okhttp3.Call
import okhttp3.Callback
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okio.Buffer

class OkHttpFetcher(
  client: OkHttpClient = OkHttpClient(),
  timeoutMs: Long = DEFAULT_TIMEOUT_MS,
  private val bufferLimitBytes: Long = DEFAULT_BUFFER_LIMIT_BYTES,
) : Fetcher {
  private val client = client.newBuilder()
    .connectTimeout(timeoutMs, TimeUnit.MILLISECONDS)
    .readTimeout(timeoutMs, TimeUnit.MILLISECONDS)
    .build()

  override suspend fun fetch(request: FetchRequest): FetchResponse {
    val call = client.newCall(request.toOkHttp())
    val response = call.await()
    val headers = response.headers.toMultimap().mapValues { it.value.last() }
    return FetchResponse(response.code.toUShort(), headers, OkHttpBody(call, response, bufferLimitBytes))
  }

  private suspend fun Call.await(): Response = suspendCancellableCoroutine { continuation ->
    continuation.invokeOnCancellation { cancel() }
    enqueue(object : Callback {
      override fun onFailure(call: Call, e: IOException) = continuation.resumeWithException(MediaException.Fetch(e.message ?: e.javaClass.simpleName))
      override fun onResponse(call: Call, response: Response) = continuation.resume(response) { _, undelivered, _ -> undelivered.close() }
    })
  }

  private fun FetchRequest.toOkHttp(): Request {
    val builder = Request.Builder().url(url)
    headers.forEach { (name, value) -> builder.header(name, value) }
    if (rangeStart != null) builder.header("Range", "bytes=$rangeStart-${rangeEnd ?: ""}")
    return builder.build()
  }

  private class OkHttpBody(private val call: Call, private val response: Response, private val limit: Long) : BodyReader {
    private val lock = Any()
    private val queue = Buffer()
    private var paused = false
    private var finished = false
    private var failure: Throwable? = null
    private var reader: CancellableContinuation<Unit>? = null
    private var pump: CancellableContinuation<Unit>? = null

    init {
      PUMPS.launch { pumpBody() }
    }

    override suspend fun nextChunk(): ByteArray? = try {
      awaitChunk()
    } catch (cancelled: CancellationException) {
      call.cancel()
      throw cancelled
    }

    override fun release() {
      call.cancel()
      synchronized(lock) { queue.clear() }
      end(null)
    }

    private suspend fun awaitChunk(): ByteArray? {
      while (true) {
        var waking: CancellableContinuation<Unit>? = null
        val bytes = synchronized(lock) {
          if (queue.size == 0L && finished) failure?.let { throw it } ?: return null
          if (queue.size == 0L) null else queue.readByteArray(minOf(queue.size, CHUNK_BYTES)).also {
            if (paused && queue.size < limit / 2) { paused = false; waking = pump; pump = null }
          }
        }
        waking?.resume(Unit)
        if (bytes != null) return bytes
        suspendCancellableCoroutine<Unit> { waiting ->
          val ready = synchronized(lock) { (queue.size > 0L || finished).also { if (!it) reader = waiting } }
          if (ready) waiting.resume(Unit)
        }
      }
    }

    private suspend fun pumpBody() {
      val source = response.body.source()
      val scratch = Buffer()
      try {
        while (true) {
          if (!awaitRoom()) return
          if (source.read(scratch, SEGMENT_BYTES) < 0) return end(null)
          val waking = synchronized(lock) { queue.writeAll(scratch); reader.also { reader = null } }
          waking?.resume(Unit)
        }
      } catch (error: IOException) {
        end(if (call.isCanceled()) CancellationException("the body read was cancelled") else MediaException.Fetch(error.message ?: error.javaClass.simpleName))
      } finally {
        response.close()
      }
    }

    private suspend fun awaitRoom(): Boolean {
      suspendCancellableCoroutine<Unit> { waiting ->
        val room = synchronized(lock) { (finished || queue.size < limit).also { if (!it) { paused = true; pump = waiting } } }
        if (room) waiting.resume(Unit)
      }
      return synchronized(lock) { !finished }
    }

    private fun end(error: Throwable?) {
      val waking = synchronized(lock) {
        if (finished) return
        finished = true
        failure = error
        listOfNotNull(reader, pump).also { reader = null; pump = null; paused = false }
      }
      waking.forEach { it.resume(Unit) }
    }
  }

  private companion object {
    const val DEFAULT_TIMEOUT_MS = 30_000L
    const val CHUNK_BYTES = 64L * 1024
    const val SEGMENT_BYTES = 8L * 1024
    const val DEFAULT_BUFFER_LIMIT_BYTES = 1L shl 20
    val PUMPS = CoroutineScope(SupervisorJob() + Dispatchers.IO)
  }
}
