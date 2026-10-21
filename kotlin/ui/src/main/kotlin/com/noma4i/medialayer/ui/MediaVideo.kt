package com.noma4i.medialayer.ui

import android.os.Handler
import android.os.Looper
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.ContentScale
import androidx.media3.ui.compose.ContentFrame
import androidx.media3.common.C
import androidx.media3.common.Player
import coil.compose.AsyncImage
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaState
import com.noma4i.medialayer.Placeholder
import com.noma4i.medialayer.isReady
import com.noma4i.medialayer.toItem
import kotlinx.coroutines.flow.flowOf

internal class VideoEdges {
  private var hasRendered = false

  fun advance(state: MediaState): Pair<Boolean, Boolean> = when (state) {
    is MediaState.Idle, is MediaState.Loading -> { hasRendered = false; false to false }
    is MediaState.Rendered -> (!hasRendered).also { hasRendered = true } to false
    is MediaState.Ended -> false to true
    else -> false to false
  }
}

internal object VideoProgress {
  const val INTERVAL_MS = 100L

  fun fraction(positionMs: ULong, durationMs: ULong?): Double? {
    if (durationMs == null || durationMs == 0uL) return null

    return minOf(positionMs.toDouble() / durationMs.toDouble(), 1.0)
  }
}

internal class PlayerProgress private constructor(private val player: Player) : Player.Listener, Runnable {
  private val main = Handler(Looper.getMainLooper())
  private val consumers = mutableMapOf<Any, (Double) -> Unit>()
  private var last: Double? = null
  private var samples = 0

  override fun onEvents(player: Player, events: Player.Events) = run()

  override fun run() {
    main.removeCallbacks(this)
    if (consumers.isEmpty()) return
    samples++
    val duration = player.duration
    if (duration != C.TIME_UNSET && duration > 0) {
      val value = (player.currentPosition.toDouble() / duration).coerceIn(0.0, 1.0)
      if (last != value) {
        last = value
        consumers.values.toList().forEach { it(value) }
      }
    }
    if (consumers.isNotEmpty() && player.isPlaying) main.postDelayed(this, VideoProgress.INTERVAL_MS)
  }

  companion object {
    private val active = mutableMapOf<Player, PlayerProgress>()
    internal val producerCount: Int get() = active.size
    internal val sampleCount: Int get() = active.values.sumOf { it.samples }

    fun subscribe(player: Player, report: (Double) -> Unit): () -> Unit {
      val demand = active.getOrPut(player) { PlayerProgress(player).also(player::addListener) }
      val token = Any()
      demand.consumers[token] = report
      demand.last?.let(report)
      demand.run()
      return {
        demand.consumers.remove(token)
        if (demand.consumers.isEmpty()) {
          demand.main.removeCallbacks(demand)
          player.removeListener(demand)
          active.remove(player)
        }
      }
    }
  }
}

@Composable
fun MediaVideo(
  source: MediaSource,
  playing: Boolean = true,
  peeking: Boolean = false,
  muted: Boolean = false,
  modifier: Modifier = Modifier,
  contentScale: ContentScale = ContentScale.Crop,
  loop: Boolean = true,
  output: MediaOutput = MediaOutput.Texture,
  onFirstFrame: () -> Unit = {},
  onEnd: () -> Unit = {},
  onProgress: ((Double) -> Unit)? = null,
  content: @Composable (MediaState) -> Unit = {}
) {
  val environment = LocalMediaEnvironment.current
  val screen = rememberScreenVisibility()
  val state = rememberPlace(source, playing, muted, loop, peeking, screen.visible)
  val edges = remember(source.url) { VideoEdges() }
  val firstFrame by rememberUpdatedState(onFirstFrame)
  val end by rememberUpdatedState(onEnd)
  val progress by rememberUpdatedState(onProgress)
  val progressPlayer by remember(environment, source.url) { environment.players?.player(source.url) ?: flowOf(null) }.collectAsState(null)
  DisposableEffect(progressPlayer, screen.visible, onProgress != null) {
    val current = progressPlayer
    if (current == null || !screen.visible || onProgress == null) return@DisposableEffect onDispose { }
    val release = PlayerProgress.subscribe(current) { progress?.invoke(it) }
    onDispose { release() }
  }
  LaunchedEffect(edges, state) {
    val (isFirstFrame, isEnd) = edges.advance(state)
    if (isFirstFrame) {
      firstFrame()
    }
    if (isEnd) end()
  }
  var view by remember(environment) { mutableStateOf<ViewHandle?>(null) }
  val player by remember(view) { view?.player ?: flowOf(null) }.collectAsState(null)
  DisposableEffect(environment, source.url, screen.visible) {
    if (!screen.visible) return@DisposableEffect onDispose { }
    val events = environment.media.playerEvents()
    val owners = environment.players?.owners
    val created = environment.players?.newView()
    view = created
    if (created != null) {
      if (owners?.claim(source.url, created.token, playing) == true) events.attach(source.url, created.token)
    }
    onDispose {
      if (created != null) {
        when (val released = owners?.release(source.url, created.token)) {
          ViewRelease.Detach -> events.detach(source.url)
          is ViewRelease.Handover -> events.attach(source.url, released.token)
          else -> Unit
        }
      }
      created?.let { environment.players?.release(it) }
      view = null
    }
  }
  DisposableEffect(environment, source.url, view, playing) {
    if (playing) view?.let { current ->
      environment.players?.owners?.prefer(source.url, current.token)?.let { environment.media.playerEvents().attach(source.url, it) }
    }
    onDispose { }
  }
  Box(modifier.then(screen.modifier)) {
    player?.let { ContentFrame(it, Modifier.fillMaxSize(), output.contentFrameType(), contentScale, true, {}) }
    if (screen.visible) VideoPlaceholder(source, state.isReady, contentScale)
    content(state)
  }
}

@Composable
private fun VideoPlaceholder(source: MediaSource, ready: Boolean, contentScale: ContentScale) {
  val media = LocalMediaEnvironment.current.media
  val placeholder by produceState<Placeholder>(Placeholder.None, media, source, ready) { value = media.placeholder(source.toItem(), ready) }
  val cover = placeholder as? Placeholder.Cover ?: return
  AsyncImage(
    model = MediaImageData(cover.url, source.headers, media.cacheKey(cover.url, source.headers)),
    contentDescription = "cover",
    imageLoader = LocalMediaEnvironment.current.imageLoader,
    contentScale = contentScale,
    modifier = Modifier.fillMaxSize()
  )
}
