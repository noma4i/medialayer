package com.noma4i.medialayer

import java.nio.file.Files
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertTrue
import org.junit.Test

class CancellationTest {
  init { NativeCallbacks.install() }

  @Test
  fun inv_ffi_09_cancelling_outer_call_drops_inner_work() = runBlocking {
    val reading = CompletableDeferred<Unit>()
    val released = CompletableDeferred<Unit>()
    val body = object : BodyReader {
      override suspend fun nextChunk(): ByteArray? {
        reading.complete(Unit)
        awaitCancellation()
      }
      override fun release() { released.complete(Unit) }
    }
    val fetcher = object : Fetcher {
      override suspend fun fetch(request: FetchRequest) = FetchResponse(200u, emptyMap(), body)
    }
    val settings = object : SettingsStore {
      override fun read(key: String): String? = null
      override fun write(key: String, value: String) {}
      override fun remove(key: String) {}
    }
    val media = MediaLayer.open(OpenConfig(availableBytes = ULong.MAX_VALUE,
      dir = Files.createTempDirectory("medialayer-cancel").toString(),
      fetcher = fetcher, settings = settings, slots = null,
      device = DeviceMemory(4096u, null, false),
      defaults = defaultLimits(), pool = defaultPoolSettings()
    ))
    try {
      val request = async { media.fetchImage("https://o.test/pending.jpg", emptyMap()) }
      withTimeout(3000) { reading.await() }
      request.cancel()
      withTimeout(3000) { released.await() }
      request.join()
      assertTrue(released.isCompleted)
    } finally {
      media.close()
    }
  }
}
