package com.noma4i.medialayer.ui

import com.noma4i.medialayer.testing.TestFetcher
import android.content.Context
import android.graphics.Bitmap
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import androidx.test.core.app.ApplicationProvider
import coil3.asImage
import coil3.memory.MemoryCache
import com.noma4i.medialayer.*
import java.nio.file.Files
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(MediaLayerTestRunner::class)
class MemoryWarningTest {
  @Test
  fun inv_memory_04_a_system_warning_clears_decoded_images() = runBlocking {
    val context = ApplicationProvider.getApplicationContext<Context>()
    val loader = mediaImageLoader(context, ImageBytes { _, _ -> com.noma4i.medialayer.ImageFetchResult(pngBytes(), com.noma4i.medialayer.ImageOrigin.NETWORK) })
    val cache = checkNotNull(loader.memoryCache)
    val dropped = CompletableDeferred<Unit>()
    val media = MediaLayer.open(OpenConfig(
      audioFocus = com.noma4i.medialayer.AudioFocus.EXCLUSIVE,
      dir = Files.createTempDirectory("memory-warning").toString(),
      fetcher = TestFetcher(),
      settings = SharedPreferencesSettings(context, "memory-warning"), slots = null,
      device = DeviceMemory(4096u, null, false), defaults = defaultLimits(), pool = defaultPoolSettings(),
      memoryReleaser = object : MemoryReleaser {
        override suspend fun shutdown() { loader.shutdown() }
        override fun dropMemoryCaches() { cache.clear(); dropped.complete(Unit) }
      }
    ))
    media.stats()
    val owner = object : LifecycleOwner { override val lifecycle = LifecycleRegistry(this) }
    val signals = SystemSignals(context, media.playerEvents(), owner.lifecycle)
    cache[MemoryCache.Key("image")] = MemoryCache.Value(Bitmap.createBitmap(8, 8, Bitmap.Config.ARGB_8888).asImage())
    assertEquals(1, cache.keys.size)

    signals.onLowMemory()

    withTimeout(10_000) { dropped.await() }
    assertEquals(0, cache.keys.size)
    loader.shutdown()
  }

  @Test
  fun systemSignalsRegisterOnceAndUnregisterOnceWhateverTheOrderOfCalls() = kotlinx.coroutines.runBlocking {
    val context = ApplicationProvider.getApplicationContext<Context>()
    val owner = object : LifecycleOwner { override val lifecycle = LifecycleRegistry(this) }
    val signals = SystemSignals(context, RecordingEvents(), owner.lifecycle)
    fun observers(): Int { org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle(); return owner.lifecycle.observerCount }

    signals.unregister()
    signals.register()
    signals.register()
    assertEquals(1, observers())
    signals.unregister()
    signals.unregister()

    assertEquals(0, observers())
  }
}
