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
  @Volatile var onReady: (UInt) -> Unit = {}

  override fun ready(slot: UInt) { calls.add("ready:$slot"); onReady(slot) }
  override fun visualReady(slot: UInt) { calls.add("visualReady:$slot") }
  override fun stalled(slot: UInt, positionMs: ULong) { calls.add("stalled:$slot:$positionMs") }
  override fun failed(slot: UInt, status: Int, detail: String) { calls.add("failed:$slot:$status") }
  override fun attach(key: String, viewToken: ULong) { calls.add("attach:$key:$viewToken") }
  override fun detach(key: String) { calls.add("detach:$key") }
  override fun ended(slot: UInt) { calls.add("ended:$slot") }
  override fun foreground() { calls.add("foreground") }
  override fun background() { calls.add("background") }
  override fun memoryWarning(level: Pressure) { calls.add("memory:$level") }
}

private fun noteMainThreadFfi(into: MutableList<String>, call: String) {
  val onMain = runCatching { android.os.Looper.myLooper() == android.os.Looper.getMainLooper() }.getOrDefault(false)
  if (onMain) into.add(call)
}

internal class RecordingCounters(private val mainThreadFfi: MutableList<String>, private val observers: MutableList<String>) : com.noma4i.medialayer.ImageCounters() {
  override fun record(work: com.noma4i.medialayer.ImageWork) {
    noteMainThreadFfi(mainThreadFfi, "record")
    if (work is com.noma4i.medialayer.ImageWork.Observe || work is com.noma4i.medialayer.ImageWork.Unobserve) observers.add(work::class.simpleName.orEmpty())
    super.record(work)
  }
}

internal fun drainFfi() = kotlinx.coroutines.runBlocking { kotlinx.coroutines.withContext(ffiQueue) { } }

internal class RecordingLayer(private val events: PlayerEvents = RecordingEvents()) : MediaLayer(NoHandle) {
  val mainThreadFfi = CopyOnWriteArrayList<String>()
  private fun noteFfi(call: String) { noteMainThreadFfi(mainThreadFfi, call) }
  override fun cacheKey(url: String, headers: Map<String, String>): String { noteFfi("cacheKey"); return url }
  val calls = CopyOnWriteArrayList<String>()
  val listeners = CopyOnWriteArrayList<Pair<String, StateListener>>()
  var closed = false
  var image: ByteArray = ByteArray(0)
  val counterWork = CopyOnWriteArrayList<String>()
  private val counters by lazy { RecordingCounters(mainThreadFfi, counterWork) }
  override fun imageStats(): com.noma4i.medialayer.ImageCounters { noteFfi("imageStats"); return counters }
  var preparedTones: TonePreparation? = null
  val toneChecks = AtomicInteger()
  var needsTone = false
  val placeholders = CopyOnWriteArrayList<Pair<String, Boolean>>()

  override fun playerEvents(): PlayerEvents = events
  val items = CopyOnWriteArrayList<MediaItem>()

  override fun visible(source: MediaItem, visible: Boolean) { noteFfi("visible"); items.add(source); calls.add("visible:${source.url}:$visible") }
  override fun setPlaying(key: String, playing: Boolean, groups: List<String>) { calls.add("playing:$key:$playing" + (if (groups.isEmpty()) "" else ":" + groups.joinToString(","))) }
  override fun setPeeking(key: String, peeking: Boolean) { calls.add("peeking:$key:$peeking") }
  override fun setMuted(key: String, muted: Boolean) { calls.add("muted:$key:$muted") }
  override fun setMutedAll(muted: Boolean) { calls.add("mutedAll:$muted") }
  override fun setRate(key: String, rate: Float) { calls.add("rate:$key:$rate") }

  override fun subscribeState(key: String, listener: StateListener): Subscription {
    listeners.add(key to listener)
    return Subscription(NoHandle)
  }

  fun emit(key: String, state: MediaState) = listeners.filter { it.first == key }.forEach { it.second.changed(state) }

  override fun itemState(key: String, state: MediaState) { noteFfi("itemState"); calls.add("itemState:$key:${state::class.simpleName}") }
  override fun seek(key: String, fraction: Double) { calls.add("seek:$key:$fraction") }
  override suspend fun fetchImage(url: String, headers: Map<String, String>): ImageFetchResult { noteFfi("fetchImage"); calls.add("fetchImage:$url"); return ImageFetchResult(image, ImageOrigin.NETWORK) }
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
