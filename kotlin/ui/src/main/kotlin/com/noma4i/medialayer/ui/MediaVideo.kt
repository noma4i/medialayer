package com.noma4i.medialayer.ui

import android.os.Handler
import android.os.Looper
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.media3.ui.compose.ContentFrame
import androidx.media3.ui.compose.SURFACE_TYPE_SURFACE_VIEW
import androidx.media3.common.C
import androidx.media3.common.Player
import coil3.compose.AsyncImage
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaState
import com.noma4i.medialayer.Placeholder
import com.noma4i.medialayer.isReady
import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.toItem
import kotlinx.coroutines.ensureActive
import java.util.concurrent.ConcurrentHashMap
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
  private val sampler = Handler(player.applicationLooper)
  private val consumers = ConcurrentHashMap<Any, (Double) -> Unit>()
  @Volatile private var last: Double? = null
  @Volatile private var samples = 0

  override fun onEvents(player: Player, events: Player.Events) = run()

  override fun run() {
    sampler.removeCallbacks(this)
    if (consumers.isEmpty()) return
    samples++
    val duration = player.duration
    if (duration != C.TIME_UNSET && duration > 0) {
      val value = (player.currentPosition.toDouble() / duration).coerceIn(0.0, 1.0)
      if (last != value) {
        last = value
        main.post { consumers.values.forEach { it(value) } }
      }
    }
    if (consumers.isNotEmpty() && player.isPlaying) sampler.postDelayed(this, VideoProgress.INTERVAL_MS)
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
      demand.sampler.post(demand)
      return {
        demand.consumers.remove(token)
        if (demand.consumers.isEmpty()) {
          demand.sampler.removeCallbacks(demand)
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
  group: String? = null,
  output: MediaOutput = MediaOutput.Texture,
  cornerRadius: Dp = 0.dp,
  alpha: Float = 1f,
  onFirstFrame: () -> Unit = {},
  onEnd: () -> Unit = {},
  onProgress: ((Double) -> Unit)? = null,
  content: @Composable (MediaState) -> Unit = {}
) {
  if (currentEnvironmentOrNull() == null) {
    Box(modifier) { content(MediaState.Idle) }
    return
  }
  val environment = LocalMediaEnvironment.current
  val screen = rememberScreenVisibility()
  var view by remember(environment, source.url) { mutableStateOf<ViewHandle?>(null) }
  val state = rememberPlace(source, playing, muted, loop, peeking, screen.visible, group, viewToken = view?.token)
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
  val player by remember(view) { view?.player ?: flowOf(null) }.collectAsState(null)
  val counters = environment.players?.videoOutputCounters
  val outputType = remember(output, alpha) { output.contentFrameType(alpha) }
  val cornerRadiusPx = with(LocalDensity.current) { cornerRadius.toPx() }
  val outputPlayer = remember(player, counters, outputType, cornerRadiusPx) {
    player?.let {
      VideoOutputPlayer(it, counters, cornerRadiusPx)
    }
  }
  DisposableEffect(environment, source.url, screen.visible) {
    if (!screen.visible) return@DisposableEffect onDispose { }
    val created = environment.players?.newView()
    view = created
    onDispose {
      created?.let { environment.players?.release(it) }
      view = null
    }
  }
  val clipped = if (cornerRadius > 0.dp) modifier.clip(RoundedCornerShape(cornerRadius)) else modifier
  val faded = if (alpha != 1f) clipped.alpha(alpha) else clipped
  Box(faded.then(screen.modifier)) {
    if (outputPlayer != null || outputType == SURFACE_TYPE_SURFACE_VIEW) {
      ContentFrame(outputPlayer, Modifier.fillMaxSize(), outputType, contentScale, true, {})
    }
    if (screen.visible) VideoPlaceholder(source.toItem(), state.isReady, contentScale)
    content(state)
  }
}

@Composable
private fun VideoPlaceholder(item: MediaItem, ready: Boolean, contentScale: ContentScale) {
  val media = LocalMediaEnvironment.current.media
  key(media, item) { VideoCover(media, item, ready, contentScale) }
}

@Composable
private fun VideoCover(media: MediaLayer, item: MediaItem, ready: Boolean, contentScale: ContentScale) {
  val placeholder by produceState<Placeholder>(Placeholder.None, ready) {
    val answer = media.placeholder(item, ready)
    ensureActive()
    value = answer
  }
  val cover = placeholder as? Placeholder.Cover ?: return
  val observer = LocalPlaceholderObserver.current
  val context = LocalContext.current
  val density = LocalDensity.current.density
  val scale = contentScale.toScale()
  val image = remember(cover.url, item.headers) { CoverSource(cover.url, item.headers) }
  BoxWithConstraints(Modifier.fillMaxSize()) {
    val pixels = decodePixels(constraints, density, image, contentScale, LocalMediaEnvironment.current.pixelSizes)
    val placementPixels = if (constraints.hasBoundedWidth && constraints.hasBoundedHeight) constraints.maxWidth.toLong() * constraints.maxHeight else 0L
    val request = remember(media, cover.url, pixels, scale, placementPixels) { imageRequest(context, image, pixels, scale, { media.cacheKey(cover.url, item.headers) }, placementPixels) }
    AsyncImage(
      model = request,
      contentDescription = "cover",
      imageLoader = LocalMediaEnvironment.current.imageLoader,
      onSuccess = { success -> observer?.invoke("cover", (success.result.request.data as? MediaImageData)?.url.orEmpty()) },
      contentScale = contentScale,
      modifier = Modifier.fillMaxSize()
    )
  }
}

private class CoverSource(override val url: String, override val headers: Map<String, String>) : MediaSource {
  override val kind: Kind get() = Kind.IMAGE
}
