package com.noma4i.medialayer.testing

import android.content.res.AssetManager
import com.noma4i.medialayer.BodyReader
import com.noma4i.medialayer.FetchRequest
import com.noma4i.medialayer.FetchResponse
import com.noma4i.medialayer.Fetcher
import com.noma4i.medialayer.MediaException
import java.io.File
import java.io.IOException
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.delay

class TestFetcher(private val chunkSize: Int = 4096) : Fetcher {
  private class Entry(val contentType: String?, val read: () -> ByteArray)

  private val entries = ConcurrentHashMap<String, Entry>()
  private val delays = ConcurrentHashMap<String, Long>()
  private val failures = ConcurrentHashMap<String, String>()
  private val seen = CopyOnWriteArrayList<FetchRequest>()

  val requests: List<FetchRequest> get() = seen.toList()

  fun requestCount(url: String): Int = seen.count { it.url == url }

  fun serve(url: String, data: ByteArray, contentType: String? = null) {
    entries[url] = Entry(contentType) { data }
  }

  fun serveFile(url: String, file: File, contentType: String? = null) {
    entries[url] = Entry(contentType) { file.readBytes() }
  }

  fun serveAsset(url: String, assets: AssetManager, name: String, contentType: String? = null) {
    entries[url] = Entry(contentType) { assets.open(name).use { it.readBytes() } }
  }

  fun delay(url: String, millis: Long) {
    delays[url] = millis
  }

  fun fail(url: String, detail: String = "offline") {
    failures[url] = detail
  }

  override suspend fun fetch(request: FetchRequest): FetchResponse {
    seen.add(request)
    delays[request.url]?.let { delay(it) }
    failures[request.url]?.let { throw MediaException.Fetch(it) }
    val entry = entries[request.url] ?: throw MediaException.Fetch("offline")
    val data = try {
      entry.read()
    } catch (error: IOException) {
      throw MediaException.Fetch("unreadable ${request.url}")
    }
    val headers = mutableMapOf("Accept-Ranges" to "bytes")
    entry.contentType?.let { headers["Content-Type"] = it }
    val start = request.rangeStart?.toLong() ?: run {
      headers["Content-Length"] = data.size.toString()
      return FetchResponse(200u, headers, TestBody(data, chunkSize))
    }
    if (start >= data.size) {
      headers["Content-Range"] = "bytes */${data.size}"
      headers["Content-Length"] = "0"
      return FetchResponse(416u, headers, TestBody(ByteArray(0), chunkSize))
    }
    val end = minOf(request.rangeEnd?.toLong() ?: Long.MAX_VALUE, data.size - 1L)
    val slice = data.copyOfRange(start.toInt(), end.toInt() + 1)
    headers["Content-Range"] = "bytes $start-$end/${data.size}"
    headers["Content-Length"] = slice.size.toString()
    return FetchResponse(206u, headers, TestBody(slice, chunkSize))
  }

  private class TestBody(data: ByteArray, private val chunkSize: Int) : BodyReader {
    private val lock = Any()
    private var rest = data

    override suspend fun nextChunk(): ByteArray? = synchronized(lock) {
      if (rest.isEmpty()) return@synchronized null
      val count = minOf(rest.size, chunkSize)
      rest.copyOfRange(0, count).also { rest = rest.copyOfRange(count, rest.size) }
    }

    override fun release() = synchronized(lock) { rest = ByteArray(0) }
  }
}
