package com.noma4i.medialayer.ui

import android.app.ActivityManager
import android.app.Application
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.LifecycleRegistry
import androidx.test.core.app.ApplicationProvider
import com.noma4i.medialayer.CacheStrategy
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MediaState
import com.noma4i.medialayer.PlaceInput
import com.noma4i.medialayer.StateListener
import com.noma4i.medialayer.isReady
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf

@RunWith(MediaLayerTestRunner::class)
class LocalFileWithoutWindowTest {
  private val app = ApplicationProvider.getApplicationContext<Application>()
  private val owner = object : LifecycleOwner {
    override val lifecycle = LifecycleRegistry.createUnsafe(this)
  }
  private val looper = shadowOf(android.os.Looper.getMainLooper())

  private fun roomyDevice() {
    val manager = app.getSystemService(ActivityManager::class.java)
    shadowOf(manager).setMemoryInfo(ActivityManager.MemoryInfo().apply { totalMem = 8L shl 30; availMem = 4L shl 30 })
    shadowOf(manager).setMemoryClass(256)
  }

  private fun untilMain(timeoutMs: Long = 10_000, condition: () -> Boolean) {
    val deadline = System.nanoTime() + timeoutMs * 1_000_000
    while (!condition()) {
      check(System.nanoTime() < deadline) { "never became true within $timeoutMs ms" }
      looper.idle()
      Thread.sleep(1)
    }
  }

  private fun open(): MediaLayer {
    roomyDevice()
    return openMediaLayer(app, settings = SharedPreferencesSettings(app), lifecycle = owner.lifecycle) { MediaLayer.open(it) }.blocking()
  }

  private class Seen(private val match: (MediaState) -> Boolean) : StateListener {
    val reached = CountDownLatch(1)
    val states = CopyOnWriteArrayList<MediaState>()
    override fun changed(state: MediaState) { states.add(state); if (match(state)) reached.countDown() }
    override fun failed(error: MediaException) = Unit
  }

  private fun image(url: String) = MediaItem(Kind.IMAGE, url, emptyMap(), null, null, strategy = CacheStrategy.WINDOW)

  private fun show(place: com.noma4i.medialayer.MediaPlace) =
    place.update(PlaceInput(true, false, null, false, null, null, false, null))

  @Test
  fun inv_api_47_a_lone_local_video_place_loads_its_slot_without_a_window() {
    roomyDevice()
    val counters = VideoOutputCounters()
    val media = openMediaLayer(app, settings = SharedPreferencesSettings(app), lifecycle = owner.lifecycle, videoOutputCounters = counters) { MediaLayer.open(it) }.blocking()
    val binding = media.binding!!
    binding.factory.mediaSourceFor = { androidx.media3.test.utils.FakeMediaSource() }
    binding.factory.onMainLooper()
    val key = "file:///bundle/clip.mp4"
    val item = MediaItem(Kind.VIDEO, key, emptyMap(), null, null, strategy = CacheStrategy.WINDOW)

    val place = media.openPlace(item)
    place.update(PlaceInput(true, true, null, false, true, null, false, null))

    untilMain { counters.snapshot()["livePlayers"] == 1L }
    assertNotNull(runBlocking { binding.factory.players.player(key).first() })
    place.release()
    media.close()
  }

  @Test
  fun inv_api_47_a_lone_local_image_place_is_declared_ready_without_a_window() {
    val media = open()
    val url = "file:///bundle/photo.png"
    val seen = Seen { it.isReady }
    val subscription = media.subscribeState(url, seen)

    val place = media.openPlace(image(url))
    show(place)

    assertTrue(seen.states.toString(), seen.reached.await(5, TimeUnit.SECONDS))
    place.release()
    subscription.close()
    media.close()
  }

  @Test
  fun inv_api_47_a_lone_local_image_place_reports_the_failure_of_its_view_without_a_window() {
    val media = open()
    val url = "file:///bundle/broken.png"
    val declared = Seen { it.isReady }
    val declaredSubscription = media.subscribeState(url, declared)

    val place = media.openPlace(image(url))
    show(place)
    assertTrue(declared.states.toString(), declared.reached.await(5, TimeUnit.SECONDS))
    val failed = Seen { it is MediaState.Failed }
    val failedSubscription = media.subscribeState(url, failed)
    place.itemState(MediaState.Failed("undecodable"))

    assertTrue(failed.states.toString(), failed.reached.await(5, TimeUnit.SECONDS))
    place.release()
    failedSubscription.close()
    declaredSubscription.close()
    media.close()
  }
}
