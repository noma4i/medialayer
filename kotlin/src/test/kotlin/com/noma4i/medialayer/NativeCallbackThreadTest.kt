package com.noma4i.medialayer

import com.sun.jna.Library
import com.sun.jna.Memory
import com.sun.jna.Native
import java.io.ByteArrayOutputStream
import java.nio.file.Files
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertFalse
import org.junit.Test

private interface StderrLibc : Library {
  fun pipe(fds: IntArray): Int
  fun dup(fd: Int): Int
  fun dup2(from: Int, to: Int): Int
  fun close(fd: Int): Int
  fun read(fd: Int, buffer: Memory, count: Long): Long
}

private const val STDERR = 2

private fun nativeStderrOf(work: () -> Unit): String {
  val libc = Native.load("c", StderrLibc::class.java)
  val pipe = IntArray(2)
  check(libc.pipe(pipe) == 0)
  val saved = libc.dup(STDERR)
  libc.dup2(pipe[1], STDERR)
  try {
    work()
  } finally {
    libc.dup2(saved, STDERR)
    libc.close(saved)
    libc.close(pipe[1])
  }
  val out = ByteArrayOutputStream()
  val buffer = Memory(4096)
  while (true) {
    val count = libc.read(pipe[0], buffer, 4096)
    if (count <= 0) break
    out.write(buffer.getByteArray(0, count.toInt()))
  }
  libc.close(pipe[0])
  return out.toString()
}

private class IdleSettings : SettingsStore {
  override fun read(key: String): String? = null
  override fun write(key: String, value: String) {}
  override fun remove(key: String) {}
}

private class IdleFetcher : Fetcher {
  override suspend fun fetch(request: FetchRequest): FetchResponse = throw MediaException.Fetch("offline")
}

class NativeCallbackThreadTest {
  @Test
  fun inv_ffi_08_a_continuation_that_resumes_inline_on_a_runtime_thread_never_detaches_the_thread_under_its_own_frames() {
    NativeCallbacks.install()
    val media = MediaLayer.open(
      OpenConfig(
        dir = Files.createTempDirectory("medialayer").toString(),
        fetcher = IdleFetcher(),
        settings = IdleSettings(),
        slots = null,
        device = DeviceMemory(totalMb = 4096u, heapClassMb = null, lowRam = false),
        defaults = defaultLimits(),
        pool = defaultPoolSettings()
      )
    )

    val stderr = nativeStderrOf {
      runBlocking(Dispatchers.Unconfined) { repeat(300) { media.stats() } }
    }

    assertFalse(stderr, stderr.contains("could not detach"))
  }
}
