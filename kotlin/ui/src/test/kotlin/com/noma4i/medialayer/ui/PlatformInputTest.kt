package com.noma4i.medialayer.ui

import com.noma4i.medialayer.CacheStrategy
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MediaWindow
import com.noma4i.medialayer.NoHandle
import com.noma4i.medialayer.WindowReadyListener
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

private class CountingWindow(private val log: CopyOnWriteArrayList<String>, private val threads: CopyOnWriteArrayList<Thread>) : MediaWindow(NoHandle) {
  @Volatile var readyListener: WindowReadyListener? = null

  override fun replace(active: UInt, sources: List<MediaItem?>) { threads.add(Thread.currentThread()); log.add("replace $active ${sources.size}") }
  override fun replacePages(active: UInt, pages: List<List<MediaItem>>) { threads.add(Thread.currentThread()); log.add("pages $active ${pages.size}") }
  override fun replaceRows(rows: List<List<MediaItem>>) { threads.add(Thread.currentThread()); log.add("rows ${rows.size}") }
  override fun showRows(first: UInt, last: UInt) { threads.add(Thread.currentThread()); log.add("show $first $last") }
  override fun subscribeReady(listener: WindowReadyListener) { readyListener = listener; TestSignal.fire() }
  override fun moveTo(active: UInt) { threads.add(Thread.currentThread()); log.add("move $active") }
  override fun setActive(isActive: Boolean) { log.add("active $isActive") }
  override fun release() { log.add("release") }
  override fun close() = Unit
}

private class WindowLayer : MediaLayer(NoHandle) {
  val log = SignallingList<String>()
  val threads = CopyOnWriteArrayList<Thread>()
  val windows = SignallingList<CountingWindow>()
  override fun openVideoWindow(): MediaWindow = CountingWindow(log, threads).also { windows.add(it) }
  override fun openImageWindow(name: String, rowsPerPage: UInt, reversed: Boolean): MediaWindow = CountingWindow(log, threads).also { windows.add(it) }
  override fun close() = Unit
}

private fun eventually(what: String, condition: () -> Boolean) {
  TestSignal.await(what, 5_000, condition)
}

private fun clip(index: Int) = MediaItem(Kind.VIDEO, "https://o.test/$index.mp4", emptyMap(), null, null, strategy = CacheStrategy.WINDOW)

class PlatformInputTest {
  @Test
  fun inv_native_20_open_objects_do_not_share_host_claims() {
    val first = SlotPlayers()
    val second = SlotPlayers()
    val ofFirst = first.newView()
    val ofSecond = second.newView()

    assertEquals(ofFirst.token, ofSecond.token)
    assertTrue(first.view(ofFirst.token) === ofFirst)
    assertTrue(second.view(ofSecond.token) === ofSecond)
    assertTrue(first.view(ofFirst.token) !== ofSecond)

    first.release(ofFirst)

    assertEquals(null, first.view(ofFirst.token))
    assertTrue(second.view(ofSecond.token) === ofSecond)
    assertEquals(1, second.liveViews)
  }

  @Test
  fun aWindowMapsRowsOffTheCallerAndAMoveDoesNotPrepareTheList() {
    val layer = WindowLayer()
    val mapped = AtomicInteger()
    val mappedOn = CopyOnWriteArrayList<Thread>()
    val follower = WindowFollower<Int>(layer, WindowLayout.Video { row -> mapped.incrementAndGet(); mappedOn.add(Thread.currentThread()); if (row == 2) null else clip(row) }) { }

    follower.replace(listOf(1, 2, 3))
    eventually("the first list") { layer.log.contains("replace 0 3") }
    val prepared = mapped.get()
    follower.move(2)
    eventually("the move") { layer.log.contains("move 2") }
    follower.replace(listOf(1, 2, 3, 4))
    eventually("the second list") { layer.log.contains("replace 2 4") }

    assertEquals(3, prepared)
    assertEquals(7, mapped.get())
    assertTrue(mappedOn.none { it === Thread.currentThread() } && layer.threads.none { it === Thread.currentThread() })
    follower.close()
    eventually("release") { layer.log.last() == "release" }
  }

  @Test
  fun aHeldMappingKeepsOneActiveAndOneLatestPendingWhileAnotherPlaceCloses() {
    val layer = RecordingLayer()
    val windows = WindowLayer()
    val entered = CountDownLatch(1)
    val release = CountDownLatch(1)
    val first = AtomicBoolean(true)
    val active = AtomicInteger()
    val maxActive = AtomicInteger()
    val follower = WindowFollower<Int>(windows, WindowLayout.Video { row ->
      maxActive.accumulateAndGet(active.incrementAndGet(), ::maxOf)
      if (first.getAndSet(false)) {
        entered.countDown()
        release.await(5, TimeUnit.SECONDS)
      }
      active.decrementAndGet()
      clip(row)
    }) { }

    follower.replace(listOf(1, 2, 3))
    assertTrue(entered.await(5, TimeUnit.SECONDS))
    val other = PlaceMount(layer, clip(9)) { }
    other.update(placeInput(visible = true))
    eventually("the other place is shown while the mapping is held") { layer.calls.contains("visible:https://o.test/9.mp4:true") }
    other.close()
    eventually("the other place is released while the mapping is held") { layer.calls.contains("visible:https://o.test/9.mp4:false") }
    for (round in 0 until 500) follower.replace(listOf(round, round + 1))
    release.countDown()

    eventually("the latest list") { windows.log.contains("replace 0 2") }
    assertEquals(listOf("replace 0 3", "replace 0 2"), windows.log.filter { it.startsWith("replace") })
    assertEquals(1, maxActive.get())
    follower.close()
  }

  @Test
  fun aCloseMergedWithARefusedUpdateStillReleasesTheCorePlace() {
    val layer = RecordingLayer()
    val updates = AtomicInteger()
    val releases = AtomicInteger()
    val closes = AtomicInteger()
    val entered = CountDownLatch(1)
    val gate = CountDownLatch(1)
    layer.placeFactory = {
      object : com.noma4i.medialayer.MediaPlace(NoHandle) {
        override fun subscribePlayback(listener: com.noma4i.medialayer.PlaybackListener) = com.noma4i.medialayer.Subscription(NoHandle)
        override fun update(input: com.noma4i.medialayer.PlaceInput) {
          updates.incrementAndGet()
          entered.countDown()
          gate.await(5, TimeUnit.SECONDS)
          throw com.noma4i.medialayer.MediaException.Overloaded("entry limit reached")
        }
        override fun seek(fraction: Double) = Unit
        override fun itemState(state: com.noma4i.medialayer.MediaState) = Unit
        override fun release() { releases.incrementAndGet() }
        override fun close() { closes.incrementAndGet(); TestSignal.fire() }
      }
    }
    val mount = PlaceMount(layer, clip(1)) { }
    mount.update(placeInput(visible = true))
    assertTrue(entered.await(5, TimeUnit.SECONDS))

    mount.update(placeInput(visible = true, playing = true))
    mount.close()
    mount.close()
    gate.countDown()

    eventually("released once") { releases.get() == 1 && closes.get() == 1 }
    assertEquals(1, updates.get())
  }

  @Test
  fun inv_ingress_43_aSeekAndACloseInOneDrainWindowSeekThePlaceBeforeItIsReleased() {
    val layer = RecordingLayer()
    val calls = SignallingList<String>()
    val entered = CountDownLatch(1)
    val gate = CountDownLatch(1)
    layer.placeFactory = {
      object : com.noma4i.medialayer.MediaPlace(NoHandle) {
        override fun subscribePlayback(listener: com.noma4i.medialayer.PlaybackListener) = com.noma4i.medialayer.Subscription(NoHandle)
        override fun update(input: com.noma4i.medialayer.PlaceInput) { calls.add("update"); entered.countDown(); gate.await(5, TimeUnit.SECONDS) }
        override fun seek(fraction: Double) { calls.add("seek $fraction") }
        override fun itemState(state: com.noma4i.medialayer.MediaState) = Unit
        override fun release() { calls.add("release") }
        override fun close() = Unit
      }
    }
    val mount = PlaceMount(layer, clip(1)) { }
    mount.update(placeInput(visible = true))
    assertTrue(entered.await(5, TimeUnit.SECONDS))

    mount.seek(0.0)
    mount.close()
    gate.countDown()

    eventually("released") { calls.lastOrNull() == "release" }
    assertEquals(listOf("update", "seek 0.0", "release"), calls.toList())
  }

  @Test
  fun aCloseBeforeTheFirstMountNeverOpensAPlace() {
    val layer = RecordingLayer()
    val opened = AtomicInteger()
    layer.placeFactory = { opened.incrementAndGet(); NullPlace() }
    val mount = PlaceMount(layer, clip(1)) { }

    mount.close()
    mount.update(placeInput(visible = true))
    drainFfi()

    assertEquals(0, opened.get())
  }

  @Test
  fun aHeldApplyOfOneDrainDoesNotDelayAnotherDrain() {
    val entered = CountDownLatch(1)
    val release = CountDownLatch(1)
    val held = LatestDrain<Int>({ _, next -> next }) { entered.countDown(); release.await(5, TimeUnit.SECONDS) }
    val applied = AtomicInteger()
    val other = LatestDrain<Int>({ _, next -> next }) { applied.incrementAndGet(); TestSignal.fire() }

    held.submit(0)
    assertTrue(entered.await(5, TimeUnit.SECONDS))
    other.submit(1)

    eventually("the other drain applies while the first is held") { applied.get() == 1 }
    release.countDown()
  }

  @Test
  fun inv_window_06_the_visible_rows_of_the_list_move_the_window_without_an_active_page() {
    val layer = WindowLayer()
    val follower = WindowFollower<Int>(layer, WindowLayout.Image("grid", 2, true) { row -> listOf(clip(row)) }) { }

    follower.replace(listOf(1, 2, 3))
    eventually("the rows") { layer.log.contains("rows 3") }
    follower.show(1..2)

    eventually("the visible rows") { layer.log.contains("show 1 2") }
    assertTrue(layer.log.none { it.startsWith("move") })
    follower.close()
  }

  @Test
  fun inv_window_05_the_readiness_of_the_core_reaches_the_view() {
    val layer = WindowLayer()
    val readings = CopyOnWriteArrayList<Boolean>()
    val follower = WindowFollower<Int>(layer, WindowLayout.Image("grid", 1, false) { row -> listOf(clip(row)) }, { readings.add(it) }) { }

    follower.replace(listOf(1))
    eventually("the window is subscribed") { layer.windows.firstOrNull()?.readyListener != null }
    layer.windows.first().readyListener!!.changed(true)

    assertEquals(listOf(true), readings.toList())
    follower.close()
  }

  @Test
  fun aPlaceMountAppliesTheFirstInputThenReleasesOnCloseWhateverFollowed() {
    val layer = RecordingLayer()
    val mount = PlaceMount(layer, clip(1)) { }
    mount.update(placeInput(visible = true, playing = true))
    eventually("the first input is applied") { layer.calls.contains("visible:https://o.test/1.mp4:true") }

    repeat(1_000) { mount.update(placeInput(visible = true, playing = it % 2 == 0)) }
    mount.close()

    eventually("released") { layer.calls.contains("visible:https://o.test/1.mp4:false") }
    assertEquals(1, layer.calls.count { it == "visible:https://o.test/1.mp4:true" })
  }

  @Test
  fun aRefusedMountReachesTheOwnerAsAFailure() {
    val layer = RecordingLayer()
    layer.placeError = com.noma4i.medialayer.MediaException.Overloaded("owner limit reached")
    val failures = SignallingList<Throwable>()
    val mount = PlaceMount(layer, clip(1)) { failures.add(it) }

    mount.update(placeInput(visible = true))

    eventually("the failure arrives") { failures.size == 1 }
    mount.close()
  }

  @Test
  fun anImageRecorderDoesNotKeepItsCountersAlive() {
    var counters: com.noma4i.medialayer.ImageCounters? = com.noma4i.medialayer.ImageCounters()
    val collected = java.lang.ref.ReferenceQueue<com.noma4i.medialayer.ImageCounters>()
    val weak = java.lang.ref.WeakReference(counters, collected)
    counters!!.recordLater(com.noma4i.medialayer.ImageWork.Observe)
    eventually("flushed") { counters!!.snapshot().activeObservers == 1uL }

    counters = null

    System.gc()
    assertTrue("image counters not collected", collected.remove(5_000) === weak)
  }

  @Test
  fun theLatestMapperServesTheNextSendAndTheReplacedOneIsReleased() {
    val base = System.nanoTime().toInt()
    var old: ((Int) -> Int)? = { it + base }
    val collected = java.lang.ref.ReferenceQueue<(Int) -> Int>()
    val weak = java.lang.ref.WeakReference(old, collected)
    val mapper = LatestMapper(old!!)
    assertEquals(1 + base, mapper(1))

    mapper.current = { it * 10 }
    old = null

    assertEquals(10, mapper(1))
    System.gc()
    assertTrue("replaced mapper not collected", collected.remove(5_000) === weak)
  }

  @Test
  fun aFailureOfAnotherItemOfTheSameUrlIsNotInherited() {
    val old = clip(1)
    val renewed = old.copy(headers = mapOf("X" to "1"))
    val failure = ScopedFailure(old, com.noma4i.medialayer.MediaState.Failed("x"))

    assertTrue(failure.of(old) != null)
    assertEquals(null, failure.of(renewed))
  }
}
