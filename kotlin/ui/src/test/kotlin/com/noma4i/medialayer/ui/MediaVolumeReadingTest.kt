package com.noma4i.medialayer.ui

import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.toList
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.After
import org.junit.runner.RunWith
import androidx.test.core.app.ApplicationProvider
import android.content.Context

private class FakeVolumePort(@Volatile var current: Float?) : VolumePort {
  val listeners = java.util.concurrent.CopyOnWriteArrayList<() -> Unit>()

  override fun level(): Float? = current

  override fun observe(onChange: () -> Unit): () -> Unit {
    listeners.add(onChange)
    TestSignal.fire()
    return { listeners.remove(onChange); TestSignal.fire() }
  }

  fun observers(count: Int) = TestSignal.await("$count volume observers", 5_000) { listeners.size == count }

  fun change(value: Float) {
    current = value
    listeners.toList().forEach { it() }
  }
}

/** The read of the change and the raise check that it queued both ran on the audio thread. */
private suspend fun AudioLink.audioThreadPassed() {
  repeat(2) {
    val passed = CompletableDeferred<Unit>()
    check(handler.post { passed.complete(Unit) })
    passed.await()
  }
}

@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(MediaLayerTestRunner::class)
class MediaVolumeReadingTest {
  private val fixtures = mutableListOf<AudioFixture>()
  private fun session(volume: VolumePort?) = AudioFixture(ApplicationProvider.getApplicationContext<Context>(), volume = volume).also { fixtures.add(it) }.link
  @After fun closeAudio() { fixtures.forEach { it.close() } }

  @Test
  fun inv_android_11_readVolume_gives_the_initial_value_and_each_change_and_cancel_unsubscribes() = runTest(UnconfinedTestDispatcher()) {
    val port = FakeVolumePort(0.4f)
    val reading = session(port).readVolume()!!
    val seen = Channel<Float>(Channel.UNLIMITED)

    assertEquals(0.4f, reading.initial)
    port.observers(0)
    val job = launch { reading.changes.collect { seen.send(it) } }
    port.observers(1)
    port.change(0.6f)
    assertEquals(0.6f, seen.receive())
    port.change(0.8f)
    assertEquals(0.8f, seen.receive())
    job.cancel()

    port.observers(0)
  }

  @Test
  fun inv_android_12_a_change_between_the_read_and_the_first_collect_is_not_lost() = runTest(UnconfinedTestDispatcher()) {
    val port = FakeVolumePort(0.4f)
    val reading = session(port).readVolume()!!

    port.change(0.9f)

    assertEquals(0.9f, reading.changes.first())
    assertNull(session(FakeVolumePort(null)).readVolume())
    assertNull(session(null).readVolume())
  }

  @Test
  fun inv_audio_10_volumeRaises_emits_on_a_rise_only_and_cancel_unsubscribes() = runTest(UnconfinedTestDispatcher()) {
    val port = FakeVolumePort(0.4f)
    val link = session(port)
    val flow = link.volumeRaises()
    val raises = Channel<Float>(Channel.UNLIMITED)

    port.observers(0)
    val job = launch { flow.collect { raises.send(port.current!!) } }
    port.observers(1)
    port.change(0.6f)
    assertEquals(0.6f, raises.receive())
    port.change(0.3f)
    link.audioThreadPassed()
    port.change(0.3005f)
    link.audioThreadPassed()
    port.change(0.5f)
    assertEquals("a fall or a step under the threshold raised the volume", 0.5f, raises.receive())
    job.cancel()

    port.observers(0)
  }

  @Test
  fun inv_audio_10_volumeRaises_is_empty_without_a_volume_range() = runTest(UnconfinedTestDispatcher()) {
    assertEquals(emptyList<Unit>(), session(FakeVolumePort(null)).volumeRaises().toList())
    assertEquals(emptyList<Unit>(), session(null).volumeRaises().toList())
  }
}
