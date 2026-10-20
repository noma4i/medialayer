package com.noma4i.medialayer.ui

import android.content.Context
import android.graphics.Bitmap
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import androidx.test.core.app.ApplicationProvider
import coil.memory.MemoryCache
import com.noma4i.medialayer.*
import java.nio.file.Files
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class MemoryWarningTest {
  @Test
  fun inv_memory_04_a_system_warning_clears_decoded_images() = runBlocking {
    val context = ApplicationProvider.getApplicationContext<Context>()
    val loader = mediaImageLoader(context, ImageBytes { _, _ -> pngBytes() })
    val cache = checkNotNull(loader.memoryCache)
    val dropped = CompletableDeferred<Unit>()
    val media = MediaLayer.open(OpenConfig(
      dir = Files.createTempDirectory("memory-warning").toString(),
      fetcher = object : Fetcher {
        override suspend fun fetch(request: FetchRequest): FetchResponse = throw MediaException.Fetch("unused origin")
      },
      settings = SharedPreferencesSettings(context, "memory-warning"), slots = null,
      device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings(),
      memoryReleaser = object : MemoryReleaser {
        override fun dropMemoryCaches() { cache.clear(); dropped.complete(Unit) }
      }
    ))
    media.stats()
    val owner = object : LifecycleOwner { override val lifecycle = LifecycleRegistry(this) }
    val signals = SystemSignals(context, media.playerEvents(), owner.lifecycle)
    cache[MemoryCache.Key("image")] = MemoryCache.Value(Bitmap.createBitmap(8, 8, Bitmap.Config.ARGB_8888))
    assertEquals(1, cache.keys.size)

    signals.onLowMemory()

    withTimeout(10_000) { dropped.await() }
    assertEquals(0, cache.keys.size)
    loader.shutdown()
  }
}
