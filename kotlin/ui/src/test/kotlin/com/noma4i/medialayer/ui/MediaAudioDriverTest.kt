package com.noma4i.medialayer.ui

import android.app.Application
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.DeviceMemory
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MicrophoneAvailability
import com.noma4i.medialayer.OpenConfig
import com.noma4i.medialayer.defaultLimits
import com.noma4i.medialayer.defaultPoolSettings
import com.noma4i.medialayer.testing.MediaAudioDriver
import com.noma4i.medialayer.testing.TestFetcher
import java.nio.file.Files
import java.util.concurrent.CopyOnWriteArrayList
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(MediaLayerTestRunner::class)
class MediaAudioDriverTest {
  private val application = ApplicationProvider.getApplicationContext<Application>()
  private val owner = object : LifecycleOwner {
    val registry: LifecycleRegistry by lazy { LifecycleRegistry.createUnsafe(this) }
    override val lifecycle: Lifecycle get() = registry
  }

  @Test
  fun inv_audio_80_the_public_driver_of_an_opened_layer_reaches_the_audio_link() {
    val layer = openMediaLayer(
      application, TestFetcher(), SharedPreferencesSettings(application, "driver"), lifecycle = owner.registry,
      audioPorts = AudioPorts(RecordingFocusPort(), TogglePermission())
    ) {
      MediaLayer.open(it.copy(dir = Files.createTempDirectory("driver").toString(), device = DeviceMemory(4096u, null, false)))
    }.blocking()
    val availability = CopyOnWriteArrayList<MicrophoneAvailability>()
    val follow = layer.binding!!.link.onAvailability { availability.add(it); TestSignal.fire() }
    val driver = MediaAudioDriver(layer)

    driver.startCall()
    TestSignal.await("external call", 5_000) { availability.lastOrNull() == MicrophoneAvailability.EXTERNAL_CALL }
    driver.endCall()
    TestSignal.await("available again", 5_000) { availability.lastOrNull() == MicrophoneAvailability.AVAILABLE }
    driver.revokePermission()
    TestSignal.await("denied", 5_000) { availability.lastOrNull() == MicrophoneAvailability.PERMISSION_DENIED }

    assertEquals(MicrophoneAvailability.PERMISSION_DENIED, availability.last())
    follow.close()
    layer.close()
  }
}
