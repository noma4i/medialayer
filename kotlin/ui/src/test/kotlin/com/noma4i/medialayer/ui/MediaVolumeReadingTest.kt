package com.noma4i.medialayer.ui

import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.flow.first
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

private class FakeVolumePort(var current: Float?) : VolumePort {
  val listeners = mutableListOf<() -> Unit>()

  override fun level(): Float? = current

  override fun observe(onChange: () -> Unit): () -> Unit {
    listeners.add(onChange)
    return { listeners.remove(onChange) }
  }

  fun change(value: Float) {
    current = value
    listeners.toList().forEach { it() }
  }
}

@OptIn(ExperimentalCoroutinesApi::class)
class MediaVolumeReadingTest {
  private fun session(volume: VolumePort?) = MediaAudioSession(MediaAudioFocus.Mix, FakeFocusPort(), 300, kotlinx.coroutines.CoroutineScope(kotlinx.coroutines.Job()), volume)

  @Test
  fun inv_android_11_readVolume_gives_the_initial_value_and_each_change_and_cancel_unsubscribes() = runTest(UnconfinedTestDispatcher()) {
    val port = FakeVolumePort(0.4f)
    val reading = session(port).readVolume()!!
    val seen = mutableListOf<Float>()

    assertEquals(0.4f, reading.initial)
    assertEquals(0, port.listeners.size)
    val job = launch { reading.changes.collect { seen.add(it) } }
    assertEquals(1, port.listeners.size)
    port.change(0.6f)
    port.change(0.8f)
    assertEquals(listOf(0.6f, 0.8f), seen)
    job.cancel()

    assertEquals(0, port.listeners.size)
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
}
