package com.noma4i.medialayer.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.runtime.collectAsState
import androidx.compose.ui.platform.LocalContext
import coil.ImageLoader
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaState
import com.noma4i.medialayer.stateFlow
import com.noma4i.medialayer.toItem

object Media {
  var muted: Boolean by mutableStateOf(false)
}

internal class MediaEnvironment(val media: MediaLayer, val players: SlotPlayers?, val imageLoader: ImageLoader)

internal val LocalMediaEnvironment = staticCompositionLocalOf<MediaEnvironment> { error("Wrap the screen in MediaLayerProvider") }

@Composable
fun MediaLayerProvider(media: MediaLayer, imageLoader: ImageLoader? = null, players: SlotPlayers? = null, content: @Composable () -> Unit) {
  val context = LocalContext.current
  val environment = remember(media, imageLoader, players) {
    val binding = media.binding
    val loader = imageLoader ?: binding?.imageLoader ?: mediaImageLoader(context, ImageBytes { url, headers -> media.fetchImage(url, headers) })
    MediaEnvironment(media, players ?: binding?.factory?.players, loader)
  }
  val muted = Media.muted
  DisposableEffect(media, muted) {
    media.setMutedAll(muted)
    onDispose { }
  }
  CompositionLocalProvider(LocalMediaEnvironment provides environment, content = content)
}

@Composable
internal fun rememberPlace(source: MediaSource, playing: Boolean?, muted: Boolean?, loop: Boolean = true): MediaState {
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
  return remember(media, item.url) { media.stateFlow(item.url) }.collectAsState(MediaState.Idle).value
}
