package com.noma4i.medialayer.ui

import android.content.Context
import android.os.HandlerThread
import android.os.Looper
import androidx.media3.common.C
import androidx.media3.common.Player
import androidx.media3.common.util.Clock
import androidx.media3.exoplayer.DefaultRenderersFactory
import androidx.media3.exoplayer.RenderersFactory
import androidx.media3.exoplayer.source.MediaSource
import androidx.media3.exoplayer.upstream.DefaultAllocator
import com.noma4i.medialayer.CreatedPlayer
import com.noma4i.medialayer.PlaybackRoute
import com.noma4i.medialayer.PlayerEventsInterface
import com.noma4i.medialayer.PlayerSlotFactory
import com.noma4i.medialayer.SlotSample
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicLong
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.emitAll
import kotlinx.coroutines.flow.flow
import okhttp3.Call
import okhttp3.OkHttpClient

class ViewHandle internal constructor(val token: ULong) {
  private val current = MutableStateFlow<Player?>(null)

  val player: StateFlow<Player?> get() = current

  internal fun show(player: Player?) {
    current.value = player
  }
}

class SlotPlayers internal constructor(val videoOutputCounters: VideoOutputCounters? = null) {
  private class Observed(val state: MutableStateFlow<Player?>, var collectors: Int = 0)
  private val observed = mutableMapOf<String, Observed>()
  private val slots = ConcurrentHashMap<String, ExoPlayerSlot>()

  fun player(key: String): Flow<Player?> = flow {
    val entry = synchronized(observed) {
      observed.getOrPut(key) { Observed(MutableStateFlow(slots[key]?.output)) }.also { it.collectors++ }
    }
    try { emitAll(entry.state) } finally {
      synchronized(observed) {
        if (--entry.collectors == 0) observed.remove(key)
      }
    }
  }

  private val views = ConcurrentHashMap<ULong, ViewHandle>()
  private val nextToken = AtomicLong(1)

  fun newView(): ViewHandle = ViewHandle(nextToken.getAndIncrement().toULong()).also { views[it.token] = it }

  fun release(view: ViewHandle) {
    views.remove(view.token)
  }

  internal fun view(token: ULong): ViewHandle? = views[token]

  internal val liveViews: Int get() = views.size

  fun snapshot(key: String): PlayerSnapshot? = slots[key]?.snapshot()

  internal fun sample(key: String): SlotSample? = slots[key]?.sample()

  internal fun route(key: String): PlaybackRoute? = slots[key]?.route

  internal fun publish(key: String, slot: ExoPlayerSlot, player: Player) = synchronized(observed) {
    slots[key] = slot
    observed[key]?.state?.value = player
  }

  internal fun retract(key: String, slot: ExoPlayerSlot) = synchronized(observed) {
    if (slots.remove(key, slot)) observed[key]?.state?.value = null
  }
}

class ExoPlayerSlotFactory(
  context: Context,
  callFactory: Call.Factory = OkHttpClient(),
  val startMuted: Boolean = false,
  val loadControl: LoadControlConfig = LoadControlConfig(),
  val audio: MediaAudioSession = MediaAudioSession(context, MediaAudioFocus.Mix),
  val videoOutputCounters: VideoOutputCounters? = null
) : PlayerSlotFactory {
  internal val appContext: Context = context.applicationContext
  internal val allocator = DefaultAllocator(true, C.DEFAULT_BUFFER_SEGMENT_SIZE)
  internal var renderersFactory: RenderersFactory = DefaultRenderersFactory(appContext).setEnableDecoderFallback(true)
  internal var clock: Clock = Clock.DEFAULT
  internal var looperFor: (UInt) -> Looper = { id -> HandlerThread("medialayer-player-$id").apply { start() }.looper }
  internal var mediaSourceFor: (PlaybackRoute) -> MediaSource = HttpMediaSources(appContext, callFactory)::create
  @Volatile internal var events: PlayerEventsInterface? = null
  private var closed = false

  val players = SlotPlayers(videoOutputCounters)

  @Synchronized
  fun bind(events: PlayerEventsInterface) {
    if (!closed) this.events = events
  }

  override suspend fun create(slot: UInt): CreatedPlayer = CreatedPlayer(createSlot(slot))

  @Synchronized
  internal fun createSlot(slot: UInt): ExoPlayerSlot = ExoPlayerSlot(slot, this)

  @Synchronized
  fun release() {
    closed = true
    events = null
    audio.close()
  }
}
