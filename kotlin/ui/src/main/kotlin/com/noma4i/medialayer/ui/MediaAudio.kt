package com.noma4i.medialayer.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaState
import kotlinx.coroutines.flow.flowOf

@Stable
class AudioControl internal constructor(playing: Boolean, private val seekTo: (Double) -> Unit) {
  private var declared = playing

  var rate: Float by mutableFloatStateOf(1f)

  var state: MediaState by mutableStateOf(MediaState.Idle)
    internal set
  var isPlaying: Boolean by mutableStateOf(playing)
    private set
  var progress: Float by mutableFloatStateOf(0f)
    internal set

  fun play() { isPlaying = true }

  fun pause() { isPlaying = false }

  fun toggle() { isPlaying = !isPlaying }

  internal fun ended() {
    isPlaying = false
    progress = 0f
  }

  internal fun follow(playing: Boolean) {
    if (declared == playing) return
    declared = playing
    isPlaying = playing
  }

  fun seek(fraction: Float) {
    if (!fraction.isFinite()) return
    val clamped = fraction.coerceIn(0f, 1f)
    progress = clamped
    seekTo(clamped.toDouble())
  }

}

@Composable
fun MediaAudio(source: MediaSource, playing: Boolean = false, muted: Boolean = false, rate: Float = 1f, group: String? = null, content: @Composable (AudioControl) -> Unit) {
  val environment = LocalMediaEnvironment.current
  val screen = rememberScreenVisibility()
  val control = remember(environment, source.url) { AudioControl(playing) { environment.media.seek(source.url, it) }.also { it.rate = rate } }
  LaunchedEffect(rate) { control.rate = rate }
  LaunchedEffect(control, playing) { control.follow(playing) }
  DisposableEffect(environment.media, source.url, control.rate) {
    environment.media.setRate(source.url, control.rate)
    onDispose { }
  }
  val state = rememberPlace(source, control.isPlaying, muted, group = group)
  if (control.state != state) control.state = state
  val player by remember(environment, source.url) { environment.players?.player(source.url) ?: flowOf(null) }.collectAsState(null)
  LaunchedEffect(state, control) { if (state == MediaState.Ended) control.ended() }
  DisposableEffect(player, screen.started, control) {
    val exo = player
    if (!screen.started || exo == null) return@DisposableEffect onDispose { }
    val release = PlayerProgress.subscribe(exo) {
      val value = it.toFloat()
      if (control.progress != value) control.progress = value
    }
    onDispose { release() }
  }
  content(control)
}
