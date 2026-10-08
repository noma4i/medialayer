package com.noma4i.medialayer.ui

import java.io.IOException
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.concurrent.LinkedBlockingQueue
import kotlin.concurrent.thread

internal class RequestHead(val line: String, val headers: Map<String, String>)

internal class TestServer(private val handler: (Socket) -> Unit) : AutoCloseable {
  private val server = ServerSocket(0, 50, InetAddress.getLoopbackAddress())
  val heads = LinkedBlockingQueue<RequestHead>()
  val url: String get() = "http://127.0.0.1:${server.localPort}/video.mp4"

  init {
    thread(isDaemon = true) {
      while (!server.isClosed) {
        val socket = try { server.accept() } catch (error: IOException) { break }
        thread(isDaemon = true) { socket.use { runCatching { serve(it) } } }
      }
    }
  }

  private fun serve(socket: Socket) {
    val reader = socket.getInputStream().bufferedReader(Charsets.ISO_8859_1)
    val line = reader.readLine() ?: return
    val headers = generateSequence { reader.readLine() }.takeWhile { it.isNotEmpty() }
      .associate { it.substringBefore(":").lowercase() to it.substringAfter(":").trim() }
    heads.add(RequestHead(line, headers))
    handler(socket)
  }

  override fun close() = server.close()
}

internal fun Socket.respond(status: String, body: String, extraHeaders: String = "") {
  val out = getOutputStream()
  out.write("HTTP/1.1 $status\r\nContent-Length: ${body.length}\r\n$extraHeaders\r\n".toByteArray())
  out.write(body.toByteArray())
  out.flush()
}

internal fun Socket.waitUntilClientLeaves() {
  val input = getInputStream()
  try { while (input.read() != -1) { } } catch (error: IOException) { }
}
