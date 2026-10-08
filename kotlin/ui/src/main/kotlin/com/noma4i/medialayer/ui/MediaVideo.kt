package com.noma4i.medialayer.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.ContentScale
import androidx.media3.ui.compose.ContentFrame
import androidx.media3.ui.compose.SURFACE_TYPE_TEXTURE_VIEW
import coil.compose.AsyncImage
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaState
import com.noma4i.medialayer.Placeholder
import com.noma4i.medialayer.toItem
import kotlinx.coroutines.flow.flowOf

@Composable
fun MediaVideo(
  source: MediaSource,
  playing: Boolean = false,
  muted: Boolean = false,
  modifier: Modifier = Modifier,
  contentScale: ContentScale = ContentScale.Crop,
  content: @Composable (MediaState) -> Unit = {}
) {
  val environment = LocalMediaEnvironment.current
  val state = rememberPlace(source, playing, muted)
  val view = remember(environment) { environment.players?.newView() }
  val player by remember(view) { view?.player ?: flowOf(null) }.collectAsState(null)
  DisposableEffect(environment, view, source.url) {
    val events = environment.media.playerEvents()
    if (view != null) events.attach(source.url, view.token)
    onDispose {
      if (view != null) events.detach(source.url)
      view?.let { environment.players?.release(it) }
    }
  }
  Box(modifier) {
    player?.let { ContentFrame(it, Modifier.fillMaxSize(), SURFACE_TYPE_TEXTURE_VIEW, contentScale, true, {}) }
    VideoPlaceholder(source, state == MediaState.Ready, contentScale)
    content(state)
  }
}

@Composable
private fun VideoPlaceholder(source: MediaSource, ready: Boolean, contentScale: ContentScale) {
  val media = LocalMediaEnvironment.current.media
  val placeholder by produceState<Placeholder>(Placeholder.None, media, source, ready) { value = media.placeholder(source.toItem(), ready) }
  val cover = placeholder as? Placeholder.Cover ?: return
  AsyncImage(
    model = MediaImageData(cover.url, source.headers),
    contentDescription = "cover",
    imageLoader = LocalMediaEnvironment.current.imageLoader,
    contentScale = contentScale,
    modifier = Modifier.fillMaxSize()
  )
}
