package com.noma4i.medialayer.ui

import com.noma4i.medialayer.BodyReader
import com.noma4i.medialayer.FetchRequest
import com.noma4i.medialayer.FetchResponse
import com.noma4i.medialayer.Fetcher
import com.noma4i.medialayer.MediaException
import java.io.IOException
import java.util.concurrent.TimeUnit
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.job
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import okhttp3.Call
import okhttp3.Callback
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response

class OkHttpFetcher(client: OkHttpClient = OkHttpClient(), timeoutMs: Long = DEFAULT_TIMEOUT_MS) : Fetcher {
  private val client = client.newBuilder()
    .connectTimeout(timeoutMs, TimeUnit.MILLISECONDS)
    .readTimeout(timeoutMs, TimeUnit.MILLISECONDS)
    .build()

  override suspend fun fetch(request: FetchRequest): FetchResponse {
    val call = client.newCall(request.toOkHttp())
    val response = call.await()
    val headers = response.headers.toMultimap().mapValues { it.value.last() }
    return FetchResponse(response.code.toUShort(), headers, OkHttpBody(call, response))
  }

  private suspend fun Call.await(): Response = suspendCancellableCoroutine { continuation ->
    continuation.invokeOnCancellation { cancel() }
    enqueue(object : Callback {
      override fun onFailure(call: Call, e: IOException) = continuation.resumeWithException(MediaException.Fetch(e.message ?: e.javaClass.simpleName))
      override fun onResponse(call: Call, response: Response) = continuation.resume(response)
    })
  }

  private fun FetchRequest.toOkHttp(): Request {
    val builder = Request.Builder().url(url)
    headers.forEach { (name, value) -> builder.header(name, value) }
    if (rangeStart != null) builder.header("Range", "bytes=$rangeStart-${rangeEnd ?: ""}")
    return builder.build()
  }

  private class OkHttpBody(private val call: Call, private val response: Response) : BodyReader {
    private val source = response.body.source()

    override suspend fun nextChunk(): ByteArray? = try {
      withContext(Dispatchers.IO) {
        val reading = coroutineContext.job
        val guard = launch(start = CoroutineStart.UNDISPATCHED) {
          try { awaitCancellation() } catch (cancelled: CancellationException) { if (reading.isCancelled) call.cancel(); throw cancelled }
        }
        try {
          read()
        } finally {
          guard.cancel()
        }
      }
    } catch (cancelled: CancellationException) {
      call.cancel()
      throw cancelled
    }

    override fun release() {
      call.cancel()
      response.close()
    }

    private fun fail(error: IOException): Nothing {
      response.close()
      if (call.isCanceled()) throw CancellationException("the body read was cancelled")
      throw MediaException.Fetch(error.message ?: error.javaClass.simpleName)
    }

    private fun read(): ByteArray? {
      val buffer = ByteArray(CHUNK_BYTES)
      val count = try { source.read(buffer) } catch (error: IOException) { fail(error) }
      if (count < 0) { response.close(); return null }
      return buffer.copyOf(count)
    }
  }

  private companion object {
    const val DEFAULT_TIMEOUT_MS = 30_000L
    const val CHUNK_BYTES = 64 * 1024
  }
}
