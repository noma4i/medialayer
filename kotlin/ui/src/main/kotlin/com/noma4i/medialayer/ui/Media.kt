package com.noma4i.medialayer.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.onVisibilityChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.findViewTreeLifecycleOwner
import coil3.ImageLoader
import com.noma4i.medialayer.MediaException
import com.noma4i.medialayer.MediaItem
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaState
import com.noma4i.medialayer.PlaybackState
import com.noma4i.medialayer.NativeFailure
import com.noma4i.medialayer.FailureSource
import com.noma4i.medialayer.stateFlow
import com.noma4i.medialayer.toItem
import com.noma4i.medialayer.normalizeItem
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.launch

object Media {
  suspend fun retry(source: MediaSource) {
    val media = ProcessMediaEnvironment.current?.media ?: throw MediaException.Closed()
    media.retry(normalizeItem(source.toItem()))
  }
  private val failureEvents = MutableSharedFlow<NativeFailure>(extraBufferCapacity = 64)
  private val failureQueue = Channel<NativeFailure>(Channel.UNLIMITED)
  private val failureScope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
  val failures = failureEvents.asSharedFlow()
  init { failureScope.launch { for (failure in failureQueue) failureEvents.emit(failure) } }
  private var acceptedMute by mutableStateOf(false)
  private data class MuteChange(val value: Boolean?, val link: AudioLink?, val layer: MediaLayer?)
  private val muteChanges = ArrayDeque<MuteChange>()
  private var changingMute = false
  private fun enqueueMute(change: MuteChange) {
    val start = synchronized(muteChanges) {
      muteChanges.addLast(change)
      if (changingMute) false else { changingMute = true; true }
    }
    if (start) applyNextMute()
  }
  private fun applyNextMute() {
    val change = synchronized(muteChanges) { muteChanges.first() }
    val value = change.value ?: acceptedMute
    val done: (Boolean) -> Unit = { accepted ->
      if (accepted && change.value != null) {
        acceptedMute = value
        ScreenAwake.shared.setMuted(value)
        onTestChange?.invoke()
      }
      val more = synchronized(muteChanges) {
        muteChanges.removeFirst()
        if (muteChanges.isEmpty()) { changingMute = false; false } else true
      }
      if (more) applyNextMute()
    }
    if (change.link != null) change.link.setMuted(value, done)
    else if (change.layer != null) inputExecutor.execute {
      val accepted = try { change.layer.setMutedAll(value); true } catch (error: Exception) { recordNativeFailure(FailureSource.SESSION, error); false }
      done(accepted)
    } else done(true)
  }
  var muted: Boolean
    get() = acceptedMute
    set(value) {
      val link = link?.takeIf { it.acceptsMute }
      val layer = ProcessMediaEnvironment.current?.media?.takeUnless { it.uniffiIsDestroyed }
      enqueueMute(MuteChange(value, link, layer))
    }

  internal fun applyMuted(layer: MediaLayer) = enqueueMute(MuteChange(null, null, layer))

  /** The last refusal of the core to take the global mute or the tone preparation, or null after a success. */
  var failure: MediaException? by mutableStateOf(null)
    internal set

  internal fun recordFailure(failure: Throwable) {
    if (failure is MediaException.Closed) return
    this.failure = failure as? MediaException ?: MediaException.Unexpected(failure.toString()).also { it.initCause(failure) }
  }

  internal fun recordNativeFailure(failure: NativeFailure) {
    if (failureEvents.subscriptionCount.value > 0) failureQueue.trySend(failure)
    recordFailure(MediaException.Unexpected(failure.message))
  }

  internal fun recordNativeFailure(source: FailureSource, failure: Throwable) =
    recordNativeFailure(NativeFailure(source, failure.javaClass.name, 0L, failure.message ?: failure.toString()))

  internal var link: AudioLink? = null
}

internal class ScreenVisibility {
  var url = ""
  private var inViewport by mutableStateOf(false)
  var started by mutableStateOf(true)
    private set

  val visible: Boolean get() = inViewport && started

  val modifier: Modifier = Modifier.onVisibilityChanged(minFractionVisible = VISIBLE_FRACTION) { deliver(it, started) }

  fun start(started: Boolean) = deliver(inViewport, started)

  private fun deliver(inViewport: Boolean, started: Boolean) {
    val before = visible
    this.inViewport = inViewport
    this.started = started
    if (visible != before) MediaTrace.emit("view") { "onScreen ${url.takeLast(24)} $visible" }
  }
}

private const val VISIBLE_FRACTION = 0.01f

@Composable
internal fun rememberScreenVisibility(url: String, stats: com.noma4i.medialayer.ImageCounters? = null): ScreenVisibility {
  val screen = remember { ScreenVisibility() }.also { it.url = url }
  val lifecycle = LocalView.current.findViewTreeLifecycleOwner()?.lifecycle
  DisposableEffect(screen, lifecycle, stats) {
    if (lifecycle == null) return@DisposableEffect onDispose { }
    val observer = LifecycleEventObserver { _, _ -> screen.start(lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)) }
    screen.start(lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED))
    lifecycle.addObserver(observer)
    stats?.recordLater(com.noma4i.medialayer.ImageWork.Observe)
    onDispose {
      lifecycle.removeObserver(observer)
      stats?.recordLater(com.noma4i.medialayer.ImageWork.Unobserve)
    }
  }
  return screen
}

internal class MediaEnvironment(val media: MediaLayer, val players: SlotPlayers?, val imageLoader: ImageLoader, val stats: com.noma4i.medialayer.ImageCounters)

private val EnvironmentLocal = staticCompositionLocalOf<MediaEnvironment?> { null }

internal object ProcessMediaEnvironment {
  private var installed: MediaEnvironment? by mutableStateOf(null)

  /** The environment of the process layer: it lives as long as the layer, not as long as a screen composition. */
  val current: MediaEnvironment?
    get() = installed?.takeUnless { it.media.uniffiIsDestroyed }

  fun install(environment: MediaEnvironment) {
    installed = environment
  }

  /** Test seam: a test process starts without a layer, as an app process does before it opens one. */
  internal fun reset() {
    installed = null
  }
}

internal object LocalMediaEnvironment {
  val current: MediaEnvironment
    @Composable get() = EnvironmentLocal.current ?: ProcessMediaEnvironment.current ?: error("Wrap the screen in MediaLayerProvider")

}

@Composable
internal fun currentEnvironmentOrNull(): MediaEnvironment? = EnvironmentLocal.current ?: ProcessMediaEnvironment.current

@Composable
fun currentMediaLayer(): MediaLayer? = (EnvironmentLocal.current ?: ProcessMediaEnvironment.current)?.media

@OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
@Composable
fun MediaLayerProvider(opened: Deferred<MediaLayer>, imageLoader: ImageLoader? = null, players: SlotPlayers? = null, content: @Composable () -> Unit) {
  var ready by remember(opened) { mutableStateOf(if (opened.isCompleted && !opened.isCancelled) opened.getCompleted() else null) }
  if (ready == null) LaunchedEffect(opened) {
    try {
      ready = opened.await()
    } catch (error: CancellationException) {
      throw error
    } catch (error: Throwable) {
      Media.recordFailure(error)
    }
  }
  MediaLayerEnvironment(ready, imageLoader, players, content)
}

@Composable
private fun MediaLayerEnvironment(media: MediaLayer?, imageLoader: ImageLoader?, players: SlotPlayers?, content: @Composable () -> Unit) {
  val context = LocalContext.current
  val environment = remember(media, imageLoader, players) {
    media?.let {
      SystemReduceMotion.install(context)
      val binding = it.binding
      val pixelSizes = PixelSizeMemo()
      val reference = java.lang.ref.WeakReference(it)
      val loader = imageLoader ?: binding?.imageLoader ?: mediaImageLoader(context, ImageBytes { url, headers ->
        val current = reference.get() ?: throw kotlinx.coroutines.CancellationException("MediaLayer was released")
        current.fetchImage(url, headers)
      }, null, it.imageStats(), pixelSizes, it.viewDecodeAdmit())
      try {
        it.setTonePreparation(ToneFrames(it, context.applicationContext, loader))
      } catch (error: MediaException) {
        Media.recordFailure(error)
      }
      MediaEnvironment(it, players ?: binding?.factory?.players, loader, it.imageStats())
    }
  }
  DisposableEffect(environment) {
    if (environment != null) {
      Media.applyMuted(environment.media)
      ProcessMediaEnvironment.install(environment)
    }
    onDispose { }
  }
  CompositionLocalProvider(EnvironmentLocal provides environment, content = content)
}

@Composable
internal fun rememberPlace(
  source: MediaSource, playing: Boolean?, muted: Boolean?, loop: Boolean, peeking: Boolean = false, visible: Boolean = true,
  group: String? = null, rate: Float? = null, viewToken: ULong? = null, mounted: MutableState<PlaceMount?>? = null,
  playGeneration: ULong = 0uL, onPlayback: (PlaybackState) -> Unit = {}
): MediaState {
  val item = source.toItem().copy(looping = loop)
  val failure = remember { mutableStateOf<ScopedFailure?>(null) }
  if (visible) declarePlace(item, playing, muted, peeking, group, rate, viewToken, mounted, playGeneration, onPlayback) { failure.value = ScopedFailure(item, MediaState.Failed(it.toString())) }
  val media = LocalMediaEnvironment.current.media
  val followed = remember(media, source.url) { mutableStateOf<MediaState>(MediaState.Idle) }
  LaunchedEffect(media, source.url, visible) {
    if (visible) media.stateFlow(source.url).collect { followed.value = it }
  }
  return failure.value.of(item) ?: followed.value
}

@Composable
internal fun declarePlace(
  item: MediaItem, playing: Boolean?, muted: Boolean?, peeking: Boolean = false, group: String? = null,
  rate: Float? = null, viewToken: ULong? = null, mounted: MutableState<PlaceMount?>? = null,
  playGeneration: ULong = 0uL, onPlayback: (PlaybackState) -> Unit = {}, onFailed: (MediaException) -> Unit = {}
) {
  val media = LocalMediaEnvironment.current.media
  val failed by rememberUpdatedState(onFailed)
  val playback by rememberUpdatedState(onPlayback)
  val view = LocalView.current
  val awake = remember(media, item) { ScreenWindows.know(view.context); ScreenAwakeHold(ScreenHoldKind.SOUND).also { it.mute(muted == true) } }
  SideEffect { awake.mute(muted == true) }
  val gate = remember(media, item) { DeliveryGate() }
  val mount = remember(media, item) {
    val token = gate.open()
    PlaceMount(media, item, { state -> gate.deliver(token) { awake.set(state.playing); playback(state) } }) { failed(it) }
  }
  DisposableEffect(mount) {
    mounted?.value = mount
    onDispose {
      gate.close()
      awake.set(false)
      mounted?.value = null
      mount.close()
    }
  }
  val input = placeInput(visible = true, playing = playing ?: false, group = group, peeking = peeking, muted = muted, rate = rate, viewToken = viewToken, playGeneration = playGeneration)
  DisposableEffect(mount, input) {
    mount.update(input)
    onDispose { }
  }
}
