package com.noma4i.medialayer.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.State
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import com.noma4i.medialayer.CameraPhase
import com.noma4i.medialayer.CameraState
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.PlayheadAnchor
import com.noma4i.medialayer.PlayheadReport
import com.noma4i.medialayer.VoicePhase
import com.noma4i.medialayer.playheadNext
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.withContext

/** The report of a take of a camera or a voice recorder: the recorded time against the limit of the take, at [nowMs]. A new take (or none) after another one is a jump. */
internal fun takeReport(recordedMs: ULong, recording: Boolean, take: ULong?, limitMs: ULong, nowMs: ULong, previousTake: ULong?): PlayheadReport = PlayheadReport(
  positionMs = recordedMs,
  durationMs = limitMs,
  atMs = nowMs,
  rate = 1.0,
  playing = recording,
  jump = take != previousTake,
  settling = false
)

/** The anchor after the reports in order, from the start of nothing: the fold the views run on every report. */
internal fun foldPlayhead(reports: List<PlayheadReport>): PlayheadAnchor =
  reports.fold(PlayheadAnchor(0.0, 0uL, 0.0)) { anchor, report -> playheadNext(anchor, report) }

private class AnchorFold {
  val reports = Channel<PlayheadReport>(Channel.UNLIMITED)
  var anchor by mutableStateOf(PlayheadAnchor(0.0, 0uL, 0.0))
}

@Composable
private fun rememberAnchorFold(key: Any?): Pair<AnchorFold, State<Float>> {
  val fold = remember(key) { AnchorFold() }
  val frame = remember(key) { mutableFloatStateOf(0f) }
  DisposableEffect(fold) { onDispose { fold.reports.close() } }
  LaunchedEffect(fold) {
    for (report in fold.reports) {
      val current = fold.anchor
      fold.anchor = withContext(Dispatchers.Default) { playheadNext(current, report) }
    }
  }
  val anchor = fold.anchor
  LaunchedEffect(fold, anchor) {
    val playhead = AudioPlayhead(anchor)
    if (anchor.perMs != 0.0) {
      while (true) withFrameNanos { frame.floatValue = playhead.progress(it) }
    } else {
      frame.floatValue = playhead.progress(System.nanoTime())
    }
  }
  return fold to frame
}

/** The position of the video a source plays, 0..1, drawn per display frame between the reports of the player. It mounts and plays nothing: no player or no length is 0. */
@Composable
fun rememberPlayheadProgress(source: MediaSource): State<Float> {
  if (currentEnvironmentOrNull() == null) return remember { mutableFloatStateOf(0f) }
  val environment = LocalMediaEnvironment.current
  val sourceKey = rememberSourceKey(source)
  val player by remember(environment, sourceKey) { sourceKey?.let { environment.players?.player(it) } ?: flowOf(null) }.collectAsState(null)
  val (fold, frame) = rememberAnchorFold(player)
  DisposableEffect(player) {
    val exo = player ?: return@DisposableEffect onDispose { }
    val release = PlayerProgress.subscribe(exo, reports = { fold.reports.trySend(it) }) { }
    onDispose { release() }
  }
  return frame
}

/** The progress of the running take of a camera against the limit of its profile, 0..1, drawn per display frame between the published states. */
@Composable
fun rememberTakeProgress(camera: MediaCamera): State<Float> {
  val (fold, frame) = rememberAnchorFold(camera)
  LaunchedEffect(fold) {
    var previous: ULong? = null
    camera.state.collect { state ->
      fold.reports.trySend(takeReport(state.recordedMs, state.phase == CameraPhase.RECORDING, state.take, camera.profile.maxDurationMs, (System.nanoTime() / 1_000_000).toULong(), previous))
      previous = state.take
    }
  }
  return frame
}

/** The progress of the running take of a voice recorder against its limit, 0..1, drawn per display frame between the published states. A recorder without a limit (0) gives 0. */
@Composable
fun rememberTakeProgress(recorder: MediaVoiceRecorder): State<Float> {
  val (fold, frame) = rememberAnchorFold(recorder)
  LaunchedEffect(fold) {
    var previous: ULong? = null
    recorder.state.collect { state ->
      fold.reports.trySend(takeReport(state.recordedMs, state.phase == VoicePhase.RECORDING, state.take, recorder.maxDurationMs, (System.nanoTime() / 1_000_000).toULong(), previous))
      previous = state.take
    }
  }
  return frame
}
