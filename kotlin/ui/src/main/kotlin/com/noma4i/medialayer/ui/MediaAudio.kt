package com.noma4i.medialayer.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.State
import androidx.compose.runtime.withFrameNanos
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaState
import com.noma4i.medialayer.Playhead
import com.noma4i.medialayer.PlayheadReport
import com.noma4i.medialayer.playheadNext
import kotlinx.coroutines.flow.flowOf

@Stable
class AudioPlayhead internal constructor(val anchor: Playhead = Playhead(0.0, 0uL, 0.0)) {
  // same arithmetic as playhead_at in the core
  fun progress(atNanos: Long): Float {
    val now = atNanos / 1_000_000
    val elapsed = if (now > anchor.atMs.toLong()) (now - anchor.atMs.toLong()).toDouble() else 0.0
    return (anchor.fraction + elapsed * anchor.perMs).coerceIn(0.0, 1.0).toFloat()
  }
}

@Stable
class AudioControl internal constructor(playing: Boolean, private val seekTo: (Double) -> Unit) {
  private var declared = playing
  private var jumpPending = false

  var rate: Float by mutableFloatStateOf(1f)

  var state: MediaState by mutableStateOf(MediaState.Idle)
    internal set
  var isPlaying: Boolean by mutableStateOf(playing)
    private set
  var progress: Float by mutableFloatStateOf(0f)
    internal set
  var playhead: AudioPlayhead by mutableStateOf(AudioPlayhead())
    private set

  fun play() { isPlaying = true }

  fun pause() { isPlaying = false }

  fun toggle() { isPlaying = !isPlaying }

  val isSounding: Boolean get() = isPlaying && (state is MediaState.Ready || state is MediaState.Rendered)

  val isBuffering: Boolean get() = isPlaying && (state is MediaState.Idle || state is MediaState.Loading)

  internal fun ended() {
    isPlaying = false
    progress = 0f
    jumpPending = true
    playhead = AudioPlayhead()
    seekTo(0.0)
  }

  internal fun report(report: PlayheadReport) {
    val next = if (!report.settling && jumpPending) report.copy(jump = true) else report
    if (!report.settling) {
      jumpPending = false
      val value = if (report.durationMs > 0uL) (report.positionMs.toDouble() / report.durationMs.toDouble()).coerceIn(0.0, 1.0).toFloat() else 0f
      if (progress != value) progress = value
    }
    playhead = AudioPlayhead(playheadNext(playhead.anchor, next))
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
    jumpPending = true
    playhead = AudioPlayhead(Playhead(clamped.toDouble(), (System.nanoTime() / 1_000_000).toULong(), playhead.anchor.perMs))
    seekTo(clamped.toDouble())
  }

}

@Composable
fun rememberPlayheadProgress(audio: AudioControl): State<Float> {
  val frame = remember(audio) { mutableFloatStateOf(0f) }
  val sounding = audio.isSounding
  val idleAnchor = if (sounding) null else audio.playhead
  LaunchedEffect(audio, sounding, idleAnchor) {
    if (sounding) {
      while (true) withFrameNanos { frame.floatValue = audio.playhead.progress(it) }
    } else {
      frame.floatValue = audio.playhead.progress(System.nanoTime())
    }
  }
  return frame
}

@Composable
fun MediaAudio(source: MediaSource, playing: Boolean = false, muted: Boolean = false, rate: Float = 1f, group: String? = null, onEnd: () -> Unit = {}, content: @Composable (AudioControl) -> Unit) {
  if (currentEnvironmentOrNull() == null) {
    content(remember { AudioControl(playing) { } })
    return
  }
  val environment = LocalMediaEnvironment.current
  val screen = rememberScreenVisibility(source.url)
  val mounted = remember(environment, source.url) { mutableStateOf<PlaceMount?>(null) }
  val control = remember(environment, source.url) { AudioControl(playing) { mounted.value?.seek(it) }.also { it.rate = rate } }
  LaunchedEffect(rate) { control.rate = rate }
  LaunchedEffect(control, playing) { control.follow(playing) }
  val state = rememberPlace(source, control.isPlaying, muted, group = group, rate = control.rate, mounted = mounted)
  if (control.state != state) control.state = state
  val sourceKey = rememberSourceKey(source)
  val player by remember(environment, sourceKey) { sourceKey?.let { environment.players?.player(it) } ?: flowOf(null) }.collectAsState(null)
  val end by rememberUpdatedState(onEnd)
  LaunchedEffect(state, control) {
    if (state != MediaState.Ended) return@LaunchedEffect
    control.ended()
    end()
  }
  DisposableEffect(player, screen.started, control) {
    val exo = player
    if (!screen.started || exo == null) return@DisposableEffect onDispose { }
    val release = PlayerProgress.subscribe(exo, reports = control::report) {
      val value = it.toFloat()
      if (control.progress != value) control.progress = value
    }
    onDispose { release() }
  }
  content(control)
}
