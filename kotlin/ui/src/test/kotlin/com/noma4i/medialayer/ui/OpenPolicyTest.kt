package com.noma4i.medialayer.ui

import android.app.Application
import android.app.ActivityManager
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.BodyReader
import com.noma4i.medialayer.Fetcher
import com.noma4i.medialayer.FetchRequest
import com.noma4i.medialayer.FetchResponse
import com.noma4i.medialayer.MediaLayer
import java.util.UUID
import java.util.concurrent.CopyOnWriteArrayList
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf

private class PolicyBody(count: Int) : BodyReader {
  private var bytes: ByteArray? = ByteArray(count) { 7 }
  override suspend fun nextChunk(): ByteArray? = bytes.also { bytes = null }
  override fun release() {}
}

private class PolicyOrigin : Fetcher {
  val seen = CopyOnWriteArrayList<FetchRequest>()

  override suspend fun fetch(request: FetchRequest): FetchResponse {
    seen.add(request)
    val count = when {
      request.url.contains("large") -> 7
      request.url.contains("new") -> 4
      else -> 2
    }
    return FetchResponse(200u, mapOf("Content-Length" to count.toString()), PolicyBody(count))
  }
}

@RunWith(RobolectricTestRunner::class)
class OpenPolicyTest {
  private val application = ApplicationProvider.getApplicationContext<Application>()

  @Before
  fun setDeviceMemory() {
    org.robolectric.shadows.ShadowStatFs.registerStats(application.cacheDir, 1_000_000, 1_000_000, 1_000_000)
    val manager = application.getSystemService(ActivityManager::class.java)
    shadowOf(manager).setMemoryInfo(ActivityManager.MemoryInfo().apply { totalMem = 4L * 1024 * 1024 * 1024 })
    shadowOf(manager).setMemoryClass(256)
  }

  private fun open(spec: String, origin: PolicyOrigin): MediaLayer =
    MediaLayer.open(application, spec, origin, SharedPreferencesSettings(application, UUID.randomUUID().toString()))

  @Test
  fun inv_spec_07_open_applies_the_first_video_forward_headers_to_origin() = runBlocking {
    val cases = listOf(
      """{"videos":[{"name":"first","forward_headers":["X-Token"]},{"name":"second","forward_headers":["Authorization"]}]}""" to mapOf("X-Token" to "selected"),
      """{"videos":[{"name":"first","forward_headers":[]}]}""" to emptyMap(),
      "{}" to mapOf("Authorization" to "excluded")
    )
    for ((spec, expected) in cases) {
      val origin = PolicyOrigin()
      val media = open(spec, origin)
      try {
        val bytes = media.fetchImage("https://o.test/${UUID.randomUUID()}/small.jpg", mapOf("X-Token" to "selected", "Authorization" to "excluded"))
        assertEquals(2, bytes.size)
        assertEquals(expected + ("Accept-Encoding" to "identity"), origin.seen.first().headers)
      } finally {
        media.release()
      }
    }
  }

  @Test
  fun inv_spec_07_open_applies_the_first_eviction_override_of_each_kind() = runBlocking {
    for ((section, suffix) in listOf("videos" to "mp4", "images" to "jpg", "audios" to "mp3")) {
      val origin = PolicyOrigin()
      val media = open("""{"cache":{"budget":10,"min":1,"evict":"lru"},"$section":[{"name":"first","evict":"largest_first"},{"name":"second","evict":"fifo"}]}""", origin)
      try {
        media.clear(null)
        val prefix = "https://o.test/${UUID.randomUUID()}"
        for (name in listOf("small", "large", "new", "small")) {
          media.fetchImage("$prefix/$name.$suffix", emptyMap())
        }
        assertEquals(1, origin.seen.count { it.url.contains("small") })
      } finally {
        media.release()
      }
    }
  }
}
