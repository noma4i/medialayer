package com.noma4i.medialayer.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
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
import coil.ImageLoader
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaState
import com.noma4i.medialayer.stateFlow
import com.noma4i.medialayer.toItem
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.flow.flowOf

object Media {
  var muted: Boolean by mutableStateOf(false)

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
    stats?.record(com.noma4i.medialayer.ImageWork.Observe)
    onDispose {
      lifecycle.removeObserver(observer)
      stats?.record(com.noma4i.medialayer.ImageWork.Unobserve)
    }
  }
  return screen
}

internal class MediaEnvironment(val media: MediaLayer, val players: SlotPlayers?, val imageLoader: ImageLoader, val stats: com.noma4i.medialayer.ImageCounters)

internal val LocalMediaEnvironment = staticCompositionLocalOf<MediaEnvironment> { error("Wrap the screen in MediaLayerProvider") }

@OptIn(kotlinx.coroutines.ExperimentalCoroutinesApi::class)
@Composable
fun MediaLayerProvider(opened: Deferred<MediaLayer>, imageLoader: ImageLoader? = null, players: SlotPlayers? = null, content: @Composable () -> Unit) {
  var ready by remember(opened) { mutableStateOf(if (opened.isCompleted && !opened.isCancelled) opened.getCompleted() else null) }
  if (ready == null) LaunchedEffect(opened) { ready = opened.await() }
  ready?.let { MediaLayerEnvironment(it, imageLoader, players, content) }
}

@Composable
private fun MediaLayerEnvironment(media: MediaLayer, imageLoader: ImageLoader?, players: SlotPlayers?, content: @Composable () -> Unit) {
  val context = LocalContext.current
  val environment = remember(media, imageLoader, players) {
    val binding = media.binding
    val reference = java.lang.ref.WeakReference(media)
    val loader = imageLoader ?: binding?.imageLoader ?: mediaImageLoader(context, ImageBytes { url, headers ->
      val current = reference.get() ?: throw kotlinx.coroutines.CancellationException("MediaLayer was released")
      current.fetchImage(url, headers)
    }, stats = media.imageStats())
    media.setTonePreparation(ToneFrames(media, context.applicationContext, loader))
    MediaEnvironment(media, players ?: binding?.factory?.players, loader, media.imageStats())
  }
  val muted = Media.muted
  DisposableEffect(media, muted) {
    media.setMutedAll(muted)
    onDispose { }
  }
  CompositionLocalProvider(LocalMediaEnvironment provides environment, content = content)
}

@Composable
internal fun rememberPlace(source: MediaSource, playing: Boolean?, muted: Boolean?, loop: Boolean = true, peeking: Boolean = false, visible: Boolean = true): MediaState {
  if (visible) declarePlace(source, playing, muted, loop, peeking)
  val media = LocalMediaEnvironment.current.media
  return remember(media, source.url, visible) { if (visible) media.stateFlow(source.url) else flowOf(MediaState.Idle) }.collectAsState(MediaState.Idle).value
}

@Composable
internal fun declarePlace(source: MediaSource, playing: Boolean?, muted: Boolean?, loop: Boolean = true, peeking: Boolean = false) {
  val media = LocalMediaEnvironment.current.media
  val item = remember(source, loop) { source.toItem().copy(looping = loop) }
  val owner = remember { Any() }
  DisposableEffect(media, item) {
    media.claims.visible(owner, item, true)
    onDispose { media.claims.release(owner, item.url) }
  }
  if (muted != null) {
    DisposableEffect(media, item.url, muted) {
      media.claims.setMuted(owner, item.url, muted)
      onDispose { }
    }
  }
  if (playing != null) {
    DisposableEffect(media, item.url, playing) {
      media.claims.setPlaying(owner, item.url, playing)
      onDispose { }
    }
  }
  if (peeking) {
    DisposableEffect(media, item.url) {
      media.claims.setPeeking(owner, item.url, true)
      onDispose { media.claims.setPeeking(owner, item.url, false) }
    }
  }
}
