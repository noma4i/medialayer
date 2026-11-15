package com.noma4i.medialayer.ui

import android.graphics.Bitmap
import android.media.MediaMetadataRetriever
import android.os.Handler
import android.os.Looper
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.staticCompositionLocalOf
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
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.layout
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.geometry.isSpecified
import androidx.media3.ui.compose.state.rememberPresentationState
import kotlin.math.roundToInt
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.media3.ui.compose.ContentFrame
import androidx.media3.ui.compose.SURFACE_TYPE_TEXTURE_VIEW
import androidx.media3.common.C
import androidx.media3.common.Player
import coil3.compose.AsyncImage
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaState
import com.noma4i.medialayer.Placeholder
import com.noma4i.medialayer.PlayheadReport
import com.noma4i.medialayer.isRendered
import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.toItem
import kotlinx.coroutines.ensureActive
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.flow.flowOf

internal class VideoEdges {
  private var hasRendered = false

  fun advance(state: MediaState): Pair<Boolean, Boolean> = when (state) {
    is MediaState.Idle -> { hasRendered = false; false to false }
    is MediaState.Rendered -> (!hasRendered).also { hasRendered = true } to false
    is MediaState.Ended -> false to true
    else -> false to false
  }
}

/** The first frame of one source: set by Rendered or Ended, it survives the screen visibility and ends with the source. */
internal class ShownFrame {
  private var url: String? = null
  private var shown = false

  fun advance(url: String, state: MediaState): Boolean {
    if (this.url != url) {
      this.url = url
      shown = false
    }
    if (state.isRendered) shown = true
    return shown
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
  private val reporters = ConcurrentHashMap<Any, (PlayheadReport) -> Unit>()
  @Volatile private var discontinuity = false
  @Volatile private var last: Double? = null
  @Volatile private var samples = 0

  override fun onPositionDiscontinuity(oldPosition: Player.PositionInfo, newPosition: Player.PositionInfo, reason: Int) {
    if (reason == Player.DISCONTINUITY_REASON_SEEK || reason == Player.DISCONTINUITY_REASON_AUTO_TRANSITION) discontinuity = true
  }

  override fun onEvents(player: Player, events: Player.Events) = run()

  override fun run() {
    sampler.removeCallbacks(this)
    if (consumers.isEmpty()) return
    samples++
    val duration = player.duration
    if (reporters.isNotEmpty()) {
      val known = duration != C.TIME_UNSET && duration > 0
      val ended = player.playbackState == Player.STATE_ENDED
      val report = PlayheadReport(
        positionMs = if (known) (if (ended) duration else player.currentPosition.coerceIn(0L, duration)).toULong() else 0uL,
        durationMs = if (known) duration.toULong() else 0uL,
        atMs = (System.nanoTime() / 1_000_000).toULong(),
        rate = player.playbackParameters.speed.toDouble(),
        playing = player.isPlaying,
        jump = discontinuity,
        settling = false
      )
      discontinuity = false
      main.post { reporters.values.forEach { it(report) } }
    }
    if (duration != C.TIME_UNSET && duration > 0) {
      val value = if (player.playbackState == Player.STATE_ENDED) 1.0 else (player.currentPosition.toDouble() / duration).coerceIn(0.0, 1.0)
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

    fun subscribe(player: Player, reports: ((PlayheadReport) -> Unit)? = null, report: (Double) -> Unit): () -> Unit {
      val demand = active.getOrPut(player) { PlayerProgress(player).also(player::addListener) }
      val token = Any()
      demand.consumers[token] = report
      reports?.let { demand.reporters[token] = it }
      demand.last?.let(report)
      demand.sampler.post(demand)
      return {
        demand.consumers.remove(token)
        demand.reporters.remove(token)
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
  val screen = rememberScreenVisibility(source.url)
  var view by remember(environment, source.url) { mutableStateOf<ViewHandle?>(null) }
  val state = rememberPlace(source, playing, muted, loop, peeking, screen.visible, group, viewToken = view?.token)
  val edges = remember(source.url) { VideoEdges() }
  val shownFrame = remember { ShownFrame() }
  val frameShown = shownFrame.advance(source.url, state)
  val firstFrame by rememberUpdatedState(onFirstFrame)
  val end by rememberUpdatedState(onEnd)
  val progress by rememberUpdatedState(onProgress)
  val sourceKey = rememberSourceKey(source)
  val progressPlayer by remember(environment, sourceKey) { sourceKey?.let { environment.players?.player(it) } ?: flowOf(null) }.collectAsState(null)
  DisposableEffect(progressPlayer, screen.visible, onProgress != null) {
    val current = progressPlayer
    if (current == null || !screen.visible || onProgress == null) return@DisposableEffect onDispose { }
    val release = PlayerProgress.subscribe(current) { progress?.invoke(it) }
    onDispose { release() }
  }
  LaunchedEffect(edges, state) {
    MediaTrace.emit("view") { "state ${source.url.takeLast(24)} $state" }
    val (isFirstFrame, isEnd) = edges.advance(state)
    if (isFirstFrame) {
      firstFrame()
    }
    if (isEnd) {
      progress?.invoke(1.0)
      end()
    }
  }
  val player by remember(view) { view?.player ?: flowOf(null) }.collectAsState(null)
  val counters = environment.players?.videoOutputCounters
  val outputPlayer = remember(player, counters) { player?.let { VideoOutputPlayer(it, counters) } }
  val surfaceHeld = remember(view) { BooleanArray(1) }
  if (outputPlayer != null) surfaceHeld[0] = true
  DisposableEffect(environment, source.url) {
    onDispose {
      view?.let { environment.players?.release(it) }
      view = null
    }
  }
  DisposableEffect(environment, source.url, screen.visible) {
    if (screen.visible && view == null) view = environment.players?.newView()
    onDispose { }
  }
  val clipped = if (cornerRadius > 0.dp) modifier.clip(RoundedCornerShape(cornerRadius)) else modifier
  val faded = if (alpha != 1f) clipped.alpha(alpha) else clipped
  Box(faded.then(screen.modifier)) {
    if (surfaceHeld[0]) {
      VideoSurface(outputPlayer, contentScale)
    }
    VideoPlaceholder(source.toItem(), frameShown, screen.visible, contentScale)
    content(state)
  }
}

/** Where the video sits in a box, box-local pixels: the one framing both the surface and the cover follow. Crop overflows the box, Fit letterboxes it. */
internal fun videoFrame(box: Size, media: Size, contentScale: ContentScale): Rect {
  if (!(media.width > 0f && media.height > 0f && box.width > 0f && box.height > 0f)) return Rect(0f, 0f, box.width, box.height)
  val factor = contentScale.computeScaleFactor(media, box)
  val width = media.width * factor.scaleX
  val height = media.height * factor.scaleY
  val left = (box.width - width) / 2f
  val top = (box.height - height) / 2f
  return Rect(left, top, left + width, top + height)
}

@Composable
private fun VideoSurface(player: Player?, contentScale: ContentScale) {
  val presentation = rememberPresentationState(player, keepContentOnReset = true)
  val density = LocalDensity.current.density
  val size = presentation.videoSizeDp
  val media = if (size != null && size.isSpecified) Size(size.width * density, size.height * density) else Size.Zero
  Box(Modifier.fillMaxSize().clipToBounds()) {
    Box(
      Modifier.layout { measurable, constraints ->
        if (!constraints.hasBoundedWidth || !constraints.hasBoundedHeight) {
          val placeable = measurable.measure(constraints)
          return@layout layout(placeable.width, placeable.height) { placeable.place(0, 0) }
        }
        val frame = videoFrame(Size(constraints.maxWidth.toFloat(), constraints.maxHeight.toFloat()), media, contentScale)
        val placeable = measurable.measure(Constraints.fixed(frame.width.roundToInt().coerceAtLeast(0), frame.height.roundToInt().coerceAtLeast(0)))
        layout(constraints.maxWidth, constraints.maxHeight) { placeable.place(frame.left.roundToInt(), frame.top.roundToInt()) }
      }
    ) {
      ContentFrame(player, Modifier.fillMaxSize(), SURFACE_TYPE_TEXTURE_VIEW, ContentScale.Fit, true, {})
    }
  }
}

/** The cover the core will answer for a remote video that has not shown a frame, known at first composition so the surface is never bare. */
internal fun provisionalCover(item: MediaItem, ready: Boolean): Placeholder {
  val cover = item.cover?.trim()?.takeIf { it.isNotEmpty() }
  val remote = item.url.trim().let { it.startsWith("http://", ignoreCase = true) || it.startsWith("https://", ignoreCase = true) }
  return if (!ready && cover != null && remote) Placeholder.Cover(cover) else Placeholder.None
}

@Composable
private fun VideoPlaceholder(item: MediaItem, ready: Boolean, enabled: Boolean, contentScale: ContentScale) {
  val media = LocalMediaEnvironment.current.media
  key(media, item) { VideoCover(media, item, ready, enabled, contentScale) }
}

@Composable
private fun VideoCover(media: MediaLayer, item: MediaItem, ready: Boolean, enabled: Boolean, contentScale: ContentScale) {
  val placeholder by produceState<Placeholder?>(null, ready, enabled) {
    if (!enabled) return@produceState
    val answer = media.placeholder(item, ready)
    ensureActive()
    value = answer
  }
  val answer = placeholder ?: if (enabled) provisionalCover(item, ready) else null
  val shown = answer as? Placeholder.Cover
  val firstFrame = answer is Placeholder.FirstFrame && item.url.trim().startsWith("file://", ignoreCase = true)
  val label = if (shown != null) "cover" else if (firstFrame) "firstframe" else "none"
  val last = remember { arrayOf<String?>(null) }
  LaunchedEffect(label, enabled) {
    if (enabled && last[0] != label) {
      last[0] = label
      MediaTrace.emit("view") { "placeholder ${item.url.takeLast(24)} $label" }
    }
  }
  if (firstFrame) {
    FirstFrameStill(item.url.trim(), contentScale)
    return
  }
  val cover = shown ?: return
  val observer = LocalPlaceholderObserver.current
  val context = LocalContext.current
  val density = LocalDensity.current.density
  val scale = contentScale.toScale()
  val image = remember(cover.url, item.headers) { CoverSource(cover.url, item.headers) }
  BoxWithConstraints(Modifier.fillMaxSize()) {
    val dimensions = decodeInput(constraints, density, image, contentScale)
    val placementPixels = if (constraints.hasBoundedWidth && constraints.hasBoundedHeight) constraints.maxWidth.toLong() * constraints.maxHeight else 0L
    val request = remember(media, cover.url, dimensions, scale, placementPixels) { imageRequest(context, image, null, scale, media, placementPixels, admit = media.viewDecodeAdmit(), dimensions = dimensions) }
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

@Composable
private fun FirstFrameStill(url: String, contentScale: ContentScale) {
  val still = LocalLocalStill.current
  BoxWithConstraints(Modifier.fillMaxSize()) {
    val width = if (constraints.hasBoundedWidth) constraints.maxWidth else 0
    val height = if (constraints.hasBoundedHeight) constraints.maxHeight else 0
    val frame by produceState<Bitmap?>(null, url, width, height) {
      value = if (width > 0 && height > 0) still.first(url, width, height) else null
    }
    frame?.let { Image(it.asImageBitmap(), contentDescription = "firstframe", contentScale = contentScale, modifier = Modifier.fillMaxSize()) }
  }
}

internal fun interface LocalStill {
  suspend fun first(url: String, widthPx: Int, heightPx: Int): Bitmap?
}

internal val LocalLocalStill = staticCompositionLocalOf<LocalStill> { MetadataStill }

internal object MetadataStill : LocalStill {
  override suspend fun first(url: String, widthPx: Int, heightPx: Int): Bitmap? {
    val retriever = MediaMetadataRetriever()
    return try {
      cancellableBlocking(abort = retriever::release) {
        try {
          retriever.setDataSource(android.net.Uri.parse(url).path ?: return@cancellableBlocking null)
          retriever.getScaledFrameAtTime(0, MediaMetadataRetriever.OPTION_CLOSEST_SYNC, widthPx, heightPx)
        } catch (_: RuntimeException) {
          null
        }
      }
    } finally {
      retriever.release()
    }
  }
}

private class CoverSource(override val url: String, override val headers: Map<String, String>) : MediaSource {
  override val kind: Kind get() = Kind.IMAGE
}
