package com.noma4i.medialayer.ui

import androidx.compose.foundation.layout.Box
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
import androidx.media3.common.C
import androidx.media3.common.Player
import androidx.media3.exoplayer.ExoPlayer
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaState
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.flowOf

@Stable
class AudioControl internal constructor(private val seekTo: (Long) -> Unit) {
  var rate: Float by mutableFloatStateOf(1f)

  var state: MediaState by mutableStateOf(MediaState.Idle)
    internal set
  var isPlaying: Boolean by mutableStateOf(false)
    private set
  var progress: Float by mutableFloatStateOf(0f)
    internal set

  fun play() { isPlaying = true }

  fun pause() { isPlaying = false }

  fun toggle() { isPlaying = !isPlaying }

  fun seek(positionMs: Long) = seekTo(positionMs)

  internal fun fractionOf(exo: ExoPlayer): Float {
    val duration = exo.duration
    return if (duration == C.TIME_UNSET || duration <= 0) 0f else (exo.currentPosition.toFloat() / duration).coerceIn(0f, 1f)
  }
}

@Composable
fun MediaAudio(source: MediaSource, muted: Boolean = false, rate: Float = 1f, content: @Composable (AudioControl) -> Unit) {
  val environment = LocalMediaEnvironment.current
  val screen = rememberScreenVisibility()
  val control = remember(environment, source.url) { AudioControl { environment.media.seek(source.url, it.toULong()) }.also { it.rate = rate } }
  LaunchedEffect(rate) { control.rate = rate }
  DisposableEffect(environment.media, source.url, control.rate) {
    environment.media.setRate(source.url, control.rate)
    onDispose { }
  }
  control.state = rememberPlace(source, control.isPlaying, muted)
  val player by remember(environment, source.url) { environment.players?.player(source.url) ?: flowOf(null) }.collectAsState(null)
  DisposableEffect(player, control) {
    val exo = player
    val listener = object : Player.Listener {
      override fun onPositionDiscontinuity(oldPosition: Player.PositionInfo, newPosition: Player.PositionInfo, reason: Int) {
        if (exo != null) control.progress = control.fractionOf(exo)
      }
      override fun onPlaybackStateChanged(playbackState: Int) {
        if (playbackState == Player.STATE_ENDED) control.pause()
      }
    }
    exo?.addListener(listener)
    onDispose { exo?.removeListener(listener) }
  }
  LaunchedEffect(player, control.isPlaying, screen.visible) {
    if (!screen.visible) return@LaunchedEffect
    val exo = player ?: return@LaunchedEffect
    while (control.isPlaying) {
      control.progress = control.fractionOf(exo)
      delay(PROGRESS_TICK_MS)
    }
    control.progress = control.fractionOf(exo)
  }
  Box(screen.modifier) { content(control) }
}

private const val PROGRESS_TICK_MS = 250L
