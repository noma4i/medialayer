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
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.runtime.collectAsState
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
import com.noma4i.medialayer.stateFlow
import com.noma4i.medialayer.toItem
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.flow.flowOf

object Media {
  var muted: Boolean by mutableStateOf(false)

  /** The last refusal of the core to take the global mute or the tone preparation, or null after a success. */
  var failure: MediaException? by mutableStateOf(null)
    internal set

  var audio: MediaAudioSession? = null
    internal set
}

internal class ScreenVisibility {
  private var inViewport by mutableStateOf(false)
  var started by mutableStateOf(true)

  val visible: Boolean get() = inViewport && started

  val modifier: Modifier = Modifier.onVisibilityChanged(minFractionVisible = VISIBLE_FRACTION) { inViewport = it }
}

private const val VISIBLE_FRACTION = 0.01f

@Composable
internal fun rememberScreenVisibility(stats: com.noma4i.medialayer.ImageCounters? = null): ScreenVisibility {
  val screen = remember { ScreenVisibility() }
  val lifecycle = LocalView.current.findViewTreeLifecycleOwner()?.lifecycle
  DisposableEffect(screen, lifecycle, stats) {
    if (lifecycle == null) return@DisposableEffect onDispose { }
    val observer = LifecycleEventObserver { _, _ -> screen.started = lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED) }
    screen.started = lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)
    lifecycle.addObserver(observer)
    stats?.recordLater(com.noma4i.medialayer.ImageWork.Observe)
    onDispose {
      lifecycle.removeObserver(observer)
      stats?.recordLater(com.noma4i.medialayer.ImageWork.Unobserve)
    }
  }
  return screen
}

internal class MediaEnvironment(val media: MediaLayer, val players: SlotPlayers?, val imageLoader: ImageLoader, val stats: com.noma4i.medialayer.ImageCounters, val pixelSizes: PixelSizeMemo)

private val EnvironmentLocal = staticCompositionLocalOf<MediaEnvironment?> { null }

internal object ProcessMediaEnvironment {
  var current: MediaEnvironment? by mutableStateOf(null)
    private set

  fun install(environment: MediaEnvironment) {
    current = environment
  }

  fun remove(environment: MediaEnvironment) {
    if (current === environment) current = null
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
      Media.failure = error as? MediaException ?: MediaException.Unexpected(error.toString())
    }
  }
  MediaLayerEnvironment(ready, imageLoader, players, content)
}

@Composable
private fun MediaLayerEnvironment(media: MediaLayer?, imageLoader: ImageLoader?, players: SlotPlayers?, content: @Composable () -> Unit) {
  val context = LocalContext.current
  val environment = remember(media, imageLoader, players) {
    media?.let {
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
        Media.failure = error
      }
      MediaEnvironment(it, players ?: binding?.factory?.players, loader, it.imageStats(), pixelSizes)
    }
  }
  val muted = Media.muted
  DisposableEffect(media, muted) {
    if (media != null) {
      try {
        media.setMutedAll(muted)
        Media.failure = null
      } catch (error: MediaException) {
        Media.failure = error
      }
    }
    onDispose { }
  }
  DisposableEffect(environment) {
    if (environment != null) ProcessMediaEnvironment.install(environment)
    onDispose { if (environment != null) ProcessMediaEnvironment.remove(environment) }
  }
  CompositionLocalProvider(EnvironmentLocal provides environment, content = content)
}

@Composable
internal fun rememberPlace(
  source: MediaSource, playing: Boolean?, muted: Boolean?, loop: Boolean = true, peeking: Boolean = false, visible: Boolean = true,
  group: String? = null, rate: Float? = null, viewToken: ULong? = null, mounted: MutableState<PlaceMount?>? = null
): MediaState {
  val item = source.toItem().copy(looping = loop)
  val failure = remember { mutableStateOf<ScopedFailure?>(null) }
  if (visible) declarePlace(item, playing, muted, peeking, group, rate, viewToken, mounted) { failure.value = ScopedFailure(item, MediaState.Failed(it.toString())) }
  val media = LocalMediaEnvironment.current.media
  val followed = remember(media, source.url, visible) { if (visible) media.stateFlow(source.url) else flowOf(MediaState.Idle) }.collectAsState(MediaState.Idle).value
  return failure.value.of(item) ?: followed
}

@Composable
internal fun declarePlace(
  item: MediaItem, playing: Boolean?, muted: Boolean?, peeking: Boolean = false, group: String? = null,
  rate: Float? = null, viewToken: ULong? = null, mounted: MutableState<PlaceMount?>? = null, onFailed: (MediaException) -> Unit = {}
) {
  val media = LocalMediaEnvironment.current.media
  val failed by rememberUpdatedState(onFailed)
  val mount = remember(media, item) { PlaceMount(media, item) { failed(it) } }
  DisposableEffect(mount) {
    mounted?.value = mount
    onDispose {
      mounted?.value = null
      mount.close()
    }
  }
  val input = placeInput(visible = true, playing = playing ?: false, group = group, peeking = peeking, muted = muted, rate = rate, viewToken = viewToken)
  DisposableEffect(mount, input) {
    mount.update(input)
    onDispose { }
  }
}
