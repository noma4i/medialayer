package com.noma4i.medialayer.ui

import android.content.Context
import androidx.media3.common.C
import androidx.media3.common.util.Clock
import androidx.media3.exoplayer.DefaultRenderersFactory
import androidx.media3.exoplayer.RenderersFactory
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.MediaSource
import androidx.media3.exoplayer.upstream.DefaultAllocator
import com.noma4i.medialayer.PlaybackRoute
import com.noma4i.medialayer.PlayerEventsInterface
import com.noma4i.medialayer.PlayerSlotFactory
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.update
import okhttp3.Call
import okhttp3.OkHttpClient

class ViewHandle internal constructor(val token: ULong) {
  private val current = MutableStateFlow<ExoPlayer?>(null)

  val player: StateFlow<ExoPlayer?> get() = current

  internal fun show(player: ExoPlayer?) {
    current.value = player
  }
}

class SlotPlayers internal constructor() {
  private val published = MutableStateFlow<Map<String, ExoPlayer>>(emptyMap())
  private val slots = ConcurrentHashMap<String, ExoPlayerSlot>()

  val keyed: StateFlow<Map<String, ExoPlayer>> get() = published

  fun player(key: String): Flow<ExoPlayer?> = published.map { it[key] }.distinctUntilChanged()

  internal val owners = ViewOwners()
  private val views = ConcurrentHashMap<ULong, ViewHandle>()
  private val nextToken = AtomicLong(1)

  fun newView(): ViewHandle = ViewHandle(nextToken.getAndIncrement().toULong()).also { views[it.token] = it }

  fun release(view: ViewHandle) {
    views.remove(view.token)
  }

  internal fun view(token: ULong): ViewHandle? = views[token]

  internal fun route(key: String): PlaybackRoute? = slots[key]?.route

  internal fun publish(key: String, slot: ExoPlayerSlot, player: ExoPlayer) {
    slots[key] = slot
    published.update { it + (key to player) }
  }

  internal fun retract(key: String, slot: ExoPlayerSlot) {
    if (!slots.remove(key, slot)) return
    published.update { it - key }
  }
}

class ExoPlayerSlotFactory(
  context: Context,
  callFactory: Call.Factory = OkHttpClient(),
  val startMuted: Boolean = false,
  val loadControl: LoadControlConfig = LoadControlConfig()
) : PlayerSlotFactory {
  internal val appContext: Context = context.applicationContext
  internal val allocator = DefaultAllocator(true, C.DEFAULT_BUFFER_SEGMENT_SIZE)
  internal var renderersFactory: RenderersFactory = DefaultRenderersFactory(appContext)
  internal var clock: Clock = Clock.DEFAULT
  internal var mediaSourceFor: (PlaybackRoute) -> MediaSource = HttpMediaSources(appContext, callFactory)::create
  @Volatile internal var events: PlayerEventsInterface? = null
  private val created = CopyOnWriteArrayList<ExoPlayerSlot>()

  val players = SlotPlayers()

  fun bind(events: PlayerEventsInterface) {
    this.events = events
  }

  override fun create(slot: UInt): ExoPlayerSlot = ExoPlayerSlot(slot, this).also { created.add(it) }

  fun release() {
    created.forEach { it.release() }
    created.clear()
  }
}
