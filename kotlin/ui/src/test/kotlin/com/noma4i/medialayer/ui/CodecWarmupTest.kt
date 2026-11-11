package com.noma4i.medialayer.ui

import android.app.ActivityManager
import android.app.Application
import android.os.Looper
import androidx.lifecycle.ProcessLifecycleOwner
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.testing.TestFetcher
import java.util.UUID
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.asCoroutineDispatcher
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

@RunWith(MediaLayerTestRunner::class)
class CodecWarmupTest {
  private val application = ApplicationProvider.getApplicationContext<Application>()
  private val warmThread = Executors.newSingleThreadExecutor { Thread(it, "codec-warmup-test") }
  private val looked = CopyOnWriteArrayList<Pair<String, String>>()
  private val release = CountDownLatch(1)
  private val done = CountDownLatch(CodecWarmup.MIMES.size)
  private val warmup = CodecWarmup(warmThread.asCoroutineDispatcher()) { mime ->
    release.await(5, TimeUnit.SECONDS)
    looked.add(mime to Thread.currentThread().name)
    done.countDown()
  }

  @Before
  fun setDeviceMemory() {
    org.robolectric.shadows.ShadowStatFs.registerStats(application.cacheDir, 1_000_000, 1_000_000, 1_000_000)
    val manager = application.getSystemService(ActivityManager::class.java)
    shadowOf(manager).setMemoryInfo(ActivityManager.MemoryInfo().apply { totalMem = 4L * 1024 * 1024 * 1024 })
    shadowOf(manager).setMemoryClass(256)
  }

  private fun open() = openMediaLayer(
    application, TestFetcher(), SharedPreferencesSettings(application, UUID.randomUUID().toString()),
    lifecycle = ProcessLifecycleOwner.get().lifecycle, codecWarmup = warmup
  ) { MediaLayer.open(it) }.blocking()

  // mutation: query the decoder list on the playback thread at the first slot again, or once per open
  // -> the first feed video pays ~0.7-1.2 s of codec init on Samsung A16 (Android cold-start profile 2026-11-12)
  @Test
  fun openingTheLibraryWarmsEachDecoderListOnceOffTheMainThreadWithoutWaiting() {
    val first = open()
    val second = open()
    assertTrue("open waited for the codec warm-up", looked.isEmpty())

    release.countDown()
    assertTrue("the warm-up did not run", done.await(5, TimeUnit.SECONDS))
    assertEquals(CodecWarmup.MIMES, looked.map { it.first })
    looked.forEach { (_, thread) -> assertNotEquals(Looper.getMainLooper().thread.name, thread.substringBefore(" @")) }
    assertEquals(setOf("codec-warmup-test"), looked.map { it.second.substringBefore(" @") }.toSet())
    first.close()
    second.close()
    warmThread.shutdownNow()
  }
}
