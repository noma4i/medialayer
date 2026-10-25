package com.noma4i.medialayer.ui

import android.app.Application
import android.app.ActivityManager
import androidx.lifecycle.ProcessLifecycleOwner
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.Evict
import com.noma4i.medialayer.Fetcher
import com.noma4i.medialayer.FetchRequest
import com.noma4i.medialayer.FetchResponse
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.OpenConfig
import com.noma4i.medialayer.SpecDefaults
import com.noma4i.medialayer.defaultLimits
import com.noma4i.medialayer.defaultPoolSettings
import java.util.UUID
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

private class OfflineOrigin : Fetcher {
  override suspend fun fetch(request: FetchRequest): FetchResponse = throw MediaException.Fetch("offline")
}

@RunWith(MediaLayerTestRunner::class)
class OpenPolicyTest {
  private val application = ApplicationProvider.getApplicationContext<Application>()

  @Before
  fun setDeviceMemory() {
    org.robolectric.shadows.ShadowStatFs.registerStats(application.cacheDir, 1_000_000, 1_000_000, 1_000_000)
    val manager = application.getSystemService(ActivityManager::class.java)
    shadowOf(manager).setMemoryInfo(ActivityManager.MemoryInfo().apply { totalMem = 4L * 1024 * 1024 * 1024 })
    shadowOf(manager).setMemoryClass(256)
  }

  @Test
  fun theForwardHeadersAndTheEvictionOverridesOfTheSpecDefaultsReachTheOpenConfig() {
    var config: OpenConfig? = null
    val resolved = SpecDefaults(
      defaultLimits(), defaultPoolSettings(), emptyList(),
      forwardHeaders = listOf("X-Token"), evictVideo = Evict.LargestFirst, evictImage = Evict.Fifo, evictAudio = Evict.Lfu
    )

    openMediaLayer(
      application, OfflineOrigin(), SharedPreferencesSettings(application, UUID.randomUUID().toString()),
      defaults = { resolved }, lifecycle = ProcessLifecycleOwner.get().lifecycle
    ) { config = it; MediaLayer.open(it) }.blocking().close()

    assertEquals(listOf("X-Token"), config!!.forwardHeaders)
    assertEquals(Evict.LargestFirst, config!!.evictVideo)
    assertEquals(Evict.Fifo, config!!.evictImage)
    assertEquals(Evict.Lfu, config!!.evictAudio)
  }
}
