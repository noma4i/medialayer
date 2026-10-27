package com.noma4i.medialayer.ui
import com.noma4i.medialayer.ImageFetchResult
import com.noma4i.medialayer.ImageOrigin

import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MediaState
import com.noma4i.medialayer.NoHandle
import com.noma4i.medialayer.PlayerEvents
import com.noma4i.medialayer.Placeholder
import com.noma4i.medialayer.Pressure
import com.noma4i.medialayer.StateListener
import com.noma4i.medialayer.Subscription
import com.noma4i.medialayer.TonePreparation
import com.noma4i.medialayer.PlaybackRoute
import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.CopyOnWriteArrayList

internal class RecordingEvents : PlayerEvents(NoHandle) {
  val calls = CopyOnWriteArrayList<String>()
  val generations = CopyOnWriteArrayList<String>()
  @Volatile var onReady: (UInt) -> Unit = {}

  override fun ready(slot: UInt, generation: ULong) { generations.add("ready:$slot:$generation"); calls.add("ready:$slot"); onReady(slot) }
  override fun visualReady(slot: UInt, generation: ULong) { generations.add("visualReady:$slot:$generation"); calls.add("visualReady:$slot") }
  override fun stalled(slot: UInt, generation: ULong, positionMs: ULong) { calls.add("stalled:$slot:$positionMs") }
  override fun failed(slot: UInt, generation: ULong, status: Int, detail: String) { generations.add("failed:$slot:$generation"); calls.add("failed:$slot:$status") }
  override fun ended(slot: UInt, generation: ULong) { generations.add("ended:$slot:$generation"); calls.add("ended:$slot") }
  override fun foreground() { calls.add("foreground") }
  override fun background() { calls.add("background") }
  override fun memoryWarning(level: Pressure) { calls.add("memory:$level") }
}

private fun noteMainThreadFfi(into: MutableList<String>, call: String) {
  val onMain = runCatching { android.os.Looper.myLooper() == android.os.Looper.getMainLooper() }.getOrDefault(false)
  if (onMain) into.add(call)
}

internal class RecordingCounters(private val mainThreadFfi: MutableList<String>, private val observers: MutableList<String>) : com.noma4i.medialayer.ImageCounters() {
  val adds = AtomicInteger()
  val observed = java.util.concurrent.atomic.AtomicLong()

  override fun record(work: com.noma4i.medialayer.ImageWork) {
    noteMainThreadFfi(mainThreadFfi, "record")
    super.record(work)
  }

  override fun add(totals: com.noma4i.medialayer.ImageTotals) {
    noteMainThreadFfi(mainThreadFfi, "record")
    adds.incrementAndGet()
    observed.addAndGet(totals.observed)
    if (totals.observed != 0L) observers.add(if (totals.observed > 0) "Observe" else "Unobserve")
    super.add(totals)
  }
}

internal fun drainFfi() {
  val deadline = System.nanoTime() + 5_000_000_000
  while (activeDrains.get() != 0 && System.nanoTime() < deadline) Thread.sleep(2)
}

internal class NullPlace : com.noma4i.medialayer.MediaPlace(NoHandle) {
  override fun update(input: com.noma4i.medialayer.PlaceInput) = Unit
  override fun seek(fraction: Double) = Unit
  override fun itemState(state: MediaState) = Unit
  override fun release() = Unit
  override fun close() = Unit
}

internal class NullWindow : com.noma4i.medialayer.MediaWindow(NoHandle) {
  override fun replace(active: UInt, sources: List<MediaItem?>) = Unit
  override fun replacePages(active: UInt, pages: List<List<MediaItem>>) = Unit
  override fun replaceRows(rows: List<List<MediaItem>>) = Unit
  override fun showRows(first: UInt, last: UInt) = Unit
  override fun subscribeReady(listener: com.noma4i.medialayer.WindowReadyListener) = Unit
  override fun moveTo(active: UInt) = Unit
  override fun setActive(isActive: Boolean) = Unit
  override fun release() = Unit
  override fun close() = Unit
}

private class RecordingPlace(private val layer: RecordingLayer, private val item: MediaItem) : com.noma4i.medialayer.MediaPlace(NoHandle) {
  private var last = com.noma4i.medialayer.PlaceInput(false, false, null, false, null, null, false, null)
  private var released = false

  override fun update(input: com.noma4i.medialayer.PlaceInput) {
    val before = last
    last = input
    val url = item.url
    if (input.visible != before.visible) { layer.items.add(item); layer.calls.add("visible:$url:${input.visible}") }
    if (input.playing != before.playing || (input.playing && input.group != before.group)) {
      layer.calls.add("playing:$url:${input.playing}" + (input.group?.let { ":$it" } ?: ""))
    }
    if (input.peeking != before.peeking) layer.calls.add("peeking:$url:${input.peeking}")
    input.muted?.takeIf { it != before.muted }?.let { layer.calls.add("muted:$url:$it") }
    input.rate?.takeIf { it != before.rate }?.let { layer.calls.add("rate:$url:$it") }
    if (input.viewToken != before.viewToken) {
      val events = layer.playerEvents() as? RecordingEvents
      events?.calls?.add(input.viewToken?.let { "attach:$url:$it" } ?: "detach:$url")
    }
  }

  override fun seek(fraction: Double) { layer.calls.add("seek:${item.url}:$fraction") }

  override fun itemState(state: MediaState) { layer.noteMain("itemState"); layer.calls.add("itemState:${item.url}:${state::class.simpleName}") }

  override fun release() {
    if (released) return
    released = true
    layer.calls.add("release:${item.url}")
    if (last.visible) layer.calls.add("visible:${item.url}:false")
  }

  override fun close() = Unit
}

internal class RecordingLayer(private val events: PlayerEvents = RecordingEvents()) : MediaLayer(NoHandle) {
  val cacheKeyReads = java.util.concurrent.atomic.AtomicInteger()
  val mainThreadFfi = CopyOnWriteArrayList<String>()
  private fun noteFfi(call: String) { noteMainThreadFfi(mainThreadFfi, call) }
  override fun cacheKey(url: String, headers: Map<String, String>): String { cacheKeyReads.incrementAndGet(); noteFfi("cacheKey"); return url }
  val calls = CopyOnWriteArrayList<String>()
  val listeners = CopyOnWriteArrayList<Pair<String, StateListener>>()
  var closed = false
  var image: ByteArray = ByteArray(0)
  val counterWork = CopyOnWriteArrayList<String>()
  private val counters by lazy { RecordingCounters(mainThreadFfi, counterWork) }
  override fun imageStats(): com.noma4i.medialayer.ImageCounters { noteFfi("imageStats"); return counters }
  val counterObserved: Long get() = counters.observed.get()
  var preparedTones: TonePreparation? = null
  val toneChecks = AtomicInteger()
  var needsTone = false
  val placeholders = CopyOnWriteArrayList<Pair<String, Boolean>>()

  override fun playerEvents(): PlayerEvents = events
  val items = CopyOnWriteArrayList<MediaItem>()

  @Volatile var placeFactory: ((MediaItem) -> com.noma4i.medialayer.MediaPlace)? = null
  @Volatile var placeError: com.noma4i.medialayer.MediaException? = null
  override fun openPlace(item: MediaItem): com.noma4i.medialayer.MediaPlace { noteFfi("openPlace"); placeError?.let { throw it }; return placeFactory?.invoke(item) ?: RecordingPlace(this, item) }
  override fun openVideoWindow(): com.noma4i.medialayer.MediaWindow = NullWindow()
  override fun openImageWindow(name: String, rowsPerPage: UInt, reversed: Boolean): com.noma4i.medialayer.MediaWindow = NullWindow()
  internal fun noteMain(call: String) = noteFfi(call)
  override fun setMutedAll(muted: Boolean) { calls.add("mutedAll:$muted") }

  override fun subscribeState(key: String, listener: StateListener): Subscription {
    listeners.add(key to listener)
    return Subscription(NoHandle)
  }

  class ToneWatch(val item: MediaItem, val band: Float, val listener: com.noma4i.medialayer.ToneListener) {
    val closed = java.util.concurrent.atomic.AtomicBoolean(false)
  }
  val toneWatches = CopyOnWriteArrayList<ToneWatch>()

  override fun subscribeTone(source: MediaItem, band: Float, listener: com.noma4i.medialayer.ToneListener): Subscription {
    noteFfi("subscribeTone")
    val watch = ToneWatch(source, band, listener)
    toneWatches.add(watch)
    return object : Subscription(NoHandle) { override fun close() { watch.closed.set(true) } }
  }

  fun emit(key: String, state: MediaState) = listeners.filter { it.first == key }.forEach { it.second.changed(state) }

  override suspend fun fetchImage(url: String, headers: Map<String, String>): ImageFetchResult { noteFfi("fetchImage"); calls.add("fetchImage:$url"); return ImageFetchResult(image, ImageOrigin.NETWORK) }
  override suspend fun reserveDecode(bytes: ULong): com.noma4i.medialayer.DecodeHold = object : com.noma4i.medialayer.DecodeHold(NoHandle) {
    override fun release() {}
  }
  override suspend fun placeholder(source: MediaItem, ready: Boolean): Placeholder {
    noteFfi("placeholder")
    placeholders.add(source.url to ready)
    return when {
      ready -> Placeholder.None
      source.kind == Kind.IMAGE -> source.blurHash?.let { Placeholder.BlurHash(it) } ?: Placeholder.None
      else -> source.cover?.let { Placeholder.Cover(it) } ?: Placeholder.FirstFrame
    }
  }

  override fun setTonePreparation(preparation: TonePreparation) { preparedTones = preparation }
  override suspend fun tonePixelsNeeded(source: MediaItem): Boolean { toneChecks.incrementAndGet(); return needsTone }
  override suspend fun playUrl(source: MediaItem): PlaybackRoute = PlaybackRoute(source.url, source.headers, null, false, false)

  override fun close() { closed = true }
}


internal fun kotlinx.coroutines.Deferred<MediaLayer>.blocking(): MediaLayer = kotlinx.coroutines.runBlocking { await() }.also { org.robolectric.Shadows.shadowOf(android.os.Looper.getMainLooper()).idle() }
