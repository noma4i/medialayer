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
import com.noma4i.medialayer.PlaybackState
import com.noma4i.medialayer.PlayheadAnchor
import com.noma4i.medialayer.PlayheadReport
import com.noma4i.medialayer.playheadNext
import kotlinx.coroutines.flow.flowOf

internal fun audioSounding(playing: Boolean, state: MediaState): Boolean =
  playing && (state is MediaState.Ready || state is MediaState.Rendered)

internal fun audioBuffering(wish: Boolean, state: MediaState): Boolean =
  wish && (state is MediaState.Idle || state is MediaState.Loading)

@Stable
class AudioPlayhead internal constructor(val anchor: PlayheadAnchor = PlayheadAnchor(0.0, 0uL, 0.0)) {
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
  internal var requested by mutableStateOf(playing)
    private set
  internal var generation by mutableStateOf(0uL)
    private set
  private var jumpPending = false
  private var playheadVersion = 0L
  private val playheadReports = ArrayDeque<Pair<Long, PlayheadReport>>()
  private var computingPlayhead = false

  var rate: Float by mutableFloatStateOf(1f)

  var state: MediaState by mutableStateOf(MediaState.Idle)
    internal set
  var playback: PlaybackState by mutableStateOf(PlaybackState(false, false, null, null, 0uL))
    private set
  val isPlaying: Boolean get() = playback.playing
  var progress: Float by mutableFloatStateOf(0f)
    internal set
  var playhead: AudioPlayhead by mutableStateOf(AudioPlayhead())
    private set

  fun play() { requested = true; generation++ }

  fun pause() { requested = false }

  fun toggle() { if (isPlaying) pause() else play() }

  val isSounding: Boolean get() = audioSounding(isPlaying, state)

  val isBuffering: Boolean get() = audioBuffering(playback.wish, state)

  internal fun ended() {
    playheadVersion++
    playheadReports.clear()
    requested = false
    progress = 0f
    jumpPending = true
    playhead = AudioPlayhead()
    seekTo(0.0)
  }

  internal fun report(report: PlayheadReport) {
    val next = if (!report.settling && jumpPending) report.copy(jump = true) else report
    MediaTrace.emit("playhead") { "report pos=${report.positionMs} dur=${report.durationMs} playing=${report.playing} rate=${report.rate} settling=${report.settling}" }
    if (!report.settling) {
      jumpPending = false
      if (report.durationMs > 0uL) {
        val value = (report.positionMs.toDouble() / report.durationMs.toDouble()).coerceIn(0.0, 1.0).toFloat()
        if (progress != value) progress = value
      }
    }
    playheadReports.addLast(playheadVersion to next)
    calculatePlayhead()
  }

  private fun calculatePlayhead() {
    if (computingPlayhead || playheadReports.isEmpty()) return
    val (version, report) = playheadReports.removeFirst()
    if (version != playheadVersion) return calculatePlayhead()
    val anchor = playhead.anchor
    computingPlayhead = true
    inputExecutor.execute {
      val result = playheadNext(anchor, report)
      android.os.Handler(android.os.Looper.getMainLooper()).post {
        if (version == playheadVersion) playhead = AudioPlayhead(result)
        computingPlayhead = false
        calculatePlayhead()
      }
    }
  }

  internal fun follow(playing: Boolean) {
    if (declared == playing) return
    declared = playing
    if (playing) play() else pause()
  }

  internal fun playback(state: PlaybackState) { playback = state }

  fun seek(fraction: Float) {
    if (!fraction.isFinite()) return
    playheadVersion++
    playheadReports.clear()
    val clamped = fraction.coerceIn(0f, 1f)
    progress = clamped
    jumpPending = true
    playhead = AudioPlayhead(PlayheadAnchor(clamped.toDouble(), (System.nanoTime() / 1_000_000).toULong(), playhead.anchor.perMs))
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
  val state = rememberPlace(source, control.requested, muted, group = group, rate = control.rate, mounted = mounted, playGeneration = control.generation, onPlayback = control::playback)
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
