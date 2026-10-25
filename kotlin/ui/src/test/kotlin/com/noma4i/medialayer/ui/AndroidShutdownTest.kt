package com.noma4i.medialayer.ui

import android.app.ActivityManager
import android.app.Application
import android.graphics.Bitmap
import android.os.Looper
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import androidx.test.core.app.ApplicationProvider
import coil3.asImage
import coil3.memory.MemoryCache
import coil3.request.ImageRequest
import coil3.request.SuccessResult
import com.noma4i.medialayer.CacheStrategy
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaLayer
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
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

  private fun roomyDevice() {
    val manager = app.getSystemService(ActivityManager::class.java)
    shadowOf(manager).setMemoryInfo(ActivityManager.MemoryInfo().apply { totalMem = 8L shl 30; availMem = 4L shl 30 })
    shadowOf(manager).setMemoryClass(256)
  }

  private fun open(): MediaLayer = openMediaLayer(app, settings = SharedPreferencesSettings(app), lifecycle = owner.lifecycle) { MediaLayer.open(it) }.blocking()

  private fun closedResources(): Pair<MediaLayer, List<java.lang.ref.WeakReference<Any>>> {
    val media = open()
    val binding = media.binding!!
    val references = listOf(binding.factory, binding.signals, binding.imageLoader).map { java.lang.ref.WeakReference(it) }
    media.close()
    untilMain { media.binding == null }
    shadowOf(Looper.getMainLooper()).idle()
    return media to references
  }

  private fun untilMain(timeoutMs: Long = 10_000, condition: () -> Boolean) {
    val deadline = System.nanoTime() + timeoutMs * 1_000_000
    while (!condition()) {
      check(System.nanoTime() < deadline) { "never became true within $timeoutMs ms" }
      shadowOf(Looper.getMainLooper()).idle()
      Thread.sleep(1)
    }
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
    loader.memoryCache!![MemoryCache.Key("held")] = MemoryCache.Value(Bitmap.createBitmap(16, 16, Bitmap.Config.ARGB_8888).asImage())
    assertTrue(loader.memoryCache!!.size > 0)
    media.close()
    untilMain { loader.memoryCache!!.size == 0L }
  }

  @Test
  fun inv_ffi_16_await_closed_on_a_live_handle_ends_after_the_real_dispose_and_a_cancelled_wait_keeps_the_cleanup() {
    for (cancelled in listOf(false, true)) {
      roomyDevice()
      val counters = VideoOutputCounters()
      val media = openMediaLayer(app, settings = SharedPreferencesSettings(app), lifecycle = owner.lifecycle, videoOutputCounters = counters) { MediaLayer.open(it) }.blocking()
      val binding = media.binding!!
      binding.factory.mediaSourceFor = { androidx.media3.test.utils.FakeMediaSource() }
      binding.factory.onMainLooper()
      media.replaceWindow(0u, listOf(MediaItem(Kind.VIDEO, "http://127.0.0.1:9/a.mp4", emptyMap(), null, null, strategy = CacheStrategy.WINDOW)))
      untilMain { counters.snapshot()["livePlayers"] == 1L }
      if (cancelled) {
        val waiting = CoroutineScope(Dispatchers.Default).launch { media.awaitClosed() }
        Thread.sleep(200)
        waiting.cancel()
        untilMain { counters.snapshot()["livePlayers"] == 0L }
      } else {
        onWorker { media.awaitClosed() }
        assertEquals(0L, counters.snapshot()["livePlayers"])
      }
      untilMain { media.binding == null }
      media.close()
    }
  }

  @Test
  fun inv_ffi_16_closed_waits_for_the_system_subscriptions_to_leave_on_the_held_main_thread() {
    val media = open()
    assertTrue(owner.lifecycle.observerCount > 0)

    val waiting = CoroutineScope(Dispatchers.Default).async { media.awaitClosed() }
    Thread.sleep(300)
    assertFalse("Closed came while the main thread still held the system subscriptions", waiting.isCompleted)
    assertTrue(owner.lifecycle.observerCount > 0)

    untilMain { waiting.isCompleted }
    assertEquals(0, owner.lifecycle.observerCount)
    media.close()
  }

  @Test
  fun inv_android_03_close_disposes_the_players_the_core_created_then_releases_callbacks_and_binding() {
    roomyDevice()
    val counters = VideoOutputCounters()
    val media = openMediaLayer(app, settings = SharedPreferencesSettings(app), lifecycle = owner.lifecycle, videoOutputCounters = counters) { MediaLayer.open(it) }.blocking()
    val binding = media.binding!!
    binding.factory.mediaSourceFor = { androidx.media3.test.utils.FakeMediaSource() }
    binding.factory.onMainLooper()
    val key = "http://127.0.0.1:9/a.mp4"
    media.replaceWindow(0u, listOf(MediaItem(Kind.VIDEO, key, emptyMap(), null, null, strategy = CacheStrategy.WINDOW)))
    untilMain { counters.snapshot()["livePlayers"] == 1L }
    assertNotNull(runBlocking { binding.factory.players.player(key).first() })
    assertTrue(owner.lifecycle.observerCount > 0)
    media.close()
    untilMain { media.binding == null }
    assertEquals(0L, counters.snapshot()["livePlayers"])
    assertEquals(0, owner.lifecycle.observerCount)
    assertNull(binding.factory.events)
    assertNull(runBlocking { binding.factory.players.player(key).first() })
  }
}
