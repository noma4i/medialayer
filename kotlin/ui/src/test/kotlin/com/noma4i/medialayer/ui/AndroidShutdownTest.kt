package com.noma4i.medialayer.ui

import android.app.Application
import android.graphics.Bitmap
import android.os.Looper
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import androidx.test.core.app.ApplicationProvider
import coil.memory.MemoryCache
import coil.request.ImageRequest
import coil.request.SuccessResult
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.PlaybackRoute
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.runBlocking
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

@RunWith(MediaLayerTestRunner::class)
class AndroidShutdownTest {
  private val app = ApplicationProvider.getApplicationContext<Application>()
  private val owner = object : LifecycleOwner {
    override val lifecycle = LifecycleRegistry.createUnsafe(this)
  }

  private fun open(): MediaLayer = openMediaLayer(app, settings = SharedPreferencesSettings(app), lifecycle = owner.lifecycle) { MediaLayer.open(it) }.blocking()

  private fun closedResources(): Pair<MediaLayer, List<java.lang.ref.WeakReference<Any>>> {
    val media = open()
    val binding = media.binding!!
    val references = listOf(binding.factory, binding.signals, binding.imageLoader).map { java.lang.ref.WeakReference(it) }
    media.close()
    shadowOf(Looper.getMainLooper()).idle()
    return media to references
  }

  @Test
  fun inv_android_09_closed_owner_does_not_retain_android_resources() {
    val (media, references) = closedResources()
    repeat(100) {
      System.gc()
      if (references.any { it.get() != null }) Thread.sleep(10)
    }
    assertEquals(emptyList<String>(), references.mapNotNull { it.get()?.javaClass?.simpleName })
    assertTrue(media.uniffiIsDestroyed)
  }

  private fun <T> onWorker(block: suspend () -> T): T {
    val result = CoroutineScope(Dispatchers.Default).async { block() }
    while (!result.isCompleted) {
      shadowOf(Looper.getMainLooper()).idle()
      Thread.sleep(1)
    }
    return runBlocking { result.await() }
  }

  @Test
  fun inv_android_01_image_events_survive_closed_owner() {
    val media = open()
    val loader = media.binding!!.imageLoader
    val bitmap = Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888)
    val request = ImageRequest.Builder(app).data(bitmap).build()
    val running = onWorker { loader.execute(request.newBuilder().listener(onStart = { media.close() }).build()) }
    assertTrue(running.toString(), running is SuccessResult)
    val result = onWorker { loader.execute(request) }
    assertTrue(result.toString(), result is SuccessResult)
    loader.shutdown()
  }

  @Test
  fun inv_android_02_close_clears_image_cache() {
    val media = open()
    val loader = media.binding!!.imageLoader
    loader.memoryCache!![MemoryCache.Key("held")] = MemoryCache.Value(Bitmap.createBitmap(16, 16, Bitmap.Config.ARGB_8888))
    assertTrue(loader.memoryCache!!.size > 0)
    media.close()
    assertEquals(0, loader.memoryCache!!.size)
  }

  @Test
  fun inv_android_03_close_releases_players_callbacks_and_binding() {
    val counters = VideoOutputCounters()
    val media = openMediaLayer(app, settings = SharedPreferencesSettings(app), lifecycle = owner.lifecycle, videoOutputCounters = counters) { MediaLayer.open(it) }.blocking()
    val binding = media.binding!!
    binding.factory.mediaSourceFor = { androidx.media3.test.utils.FakeMediaSource() }
    val slot = binding.factory.create(0u)
    slot.load("held", PlaybackRoute("http://127.0.0.1:9/a.mp4", emptyMap(), null, false, false))
    shadowOf(Looper.getMainLooper()).idle()
    assertEquals(1L, counters.snapshot()["livePlayers"])
    assertTrue(owner.lifecycle.observerCount > 0)
    media.close()
    shadowOf(Looper.getMainLooper()).idle()
    assertEquals(0L, counters.snapshot()["livePlayers"])
    assertEquals(0, owner.lifecycle.observerCount)
    assertNull(binding.factory.events)
    assertNull(media.binding)
    assertNull(slot.player)
  }
}
