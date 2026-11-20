package com.noma4i.medialayer.ui

import android.os.Build
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.blur
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.MediaSource
import com.noma4i.medialayer.MediaState
import com.noma4i.medialayer.OutgoingFailure
import com.noma4i.medialayer.OutgoingPhase
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/** What is drawn before the remote copy is ready. Local, then the blur hash, then the app's custom cover, in that order of preference. */
sealed interface OutgoingCover {
  /** The local original, sharp. */
  data object Local : OutgoingCover
  /** The local original blurred (plain below API 31), only before `ready`. */
  data class BlurredLocal(val radius: Dp = 12.dp) : OutgoingCover
  /** The app's own cover. */
  data class Custom(val content: @Composable () -> Unit) : OutgoingCover
}

private data class OutgoingRemoteSource(override val url: String) : MediaSource {
  override val kind: Kind get() = Kind.IMAGE
  override val headers: Map<String, String> get() = emptyMap()
}

/** The phase as text for the semantics (`stateDescription`): sending, awaitingRemote, fetchingRemote, ready, failed.upload, failed.remote. */
fun OutgoingPhase.describe(): String = when (this) {
  is OutgoingPhase.Sending -> "sending"
  is OutgoingPhase.AwaitingRemote -> "awaitingRemote"
  is OutgoingPhase.FetchingRemote -> "fetchingRemote"
  is OutgoingPhase.Ready -> "ready"
  is OutgoingPhase.Failed -> if (reason is OutgoingFailure.Upload) "failed.upload" else "failed.remote"
}

/**
 * Draws an outgoing item: the cover of the local original (or the blur hash) while it is sent and fetched, then the remote copy swapped in
 * once, in one frame, no crossfade. The phase is in the semantics: `stateDescription` and `testTag("outgoing-<key>")`.
 */
@Composable
fun OutgoingMediaImage(
  item: OutgoingMedia,
  modifier: Modifier = Modifier,
  contentScale: ContentScale = ContentScale.Crop,
  cover: OutgoingCover = OutgoingCover.Local,
  loader: @Composable (phase: OutgoingPhase, hasLocal: Boolean) -> Unit = { _, _ -> },
  failure: @Composable (reason: OutgoingFailure) -> Unit = {}
) {
  val phase by item.phases.collectAsState()
  val screen = rememberScreenVisibility(item.key)
  DisposableEffect(item) {
    onDispose { }
  }
  LaunchedEffect(item, screen) {
    var seen = false
    try {
      snapshotFlow { screen.visible }.collect { visible -> if (visible != seen) { seen = visible; item.onScreen(visible) } }
    } finally {
      if (seen) item.onScreen(false)
    }
  }
  var remoteShown by remember(item) { mutableStateOf(false) }
  Box(
    modifier.then(screen.modifier).clipToBounds().testTag("outgoing-${item.key}").semantics { stateDescription = phase.describe() }
  ) {
    if (!remoteShown) Cover(item, cover, contentScale)
    val url = item.remote
    if (phase == OutgoingPhase.Ready && url != null) {
      MediaImage(OutgoingRemoteSource(url), Modifier.fillMaxSize(), contentScale) { state -> if (state == MediaState.Ready) remoteShown = true }
    }
    if (phase.isBusy) loader(phase, item.hasLocal)
    (phase as? OutgoingPhase.Failed)?.let { failure(it.reason) }
  }
}

@Composable
private fun Cover(item: OutgoingMedia, cover: OutgoingCover, contentScale: ContentScale) {
  if (cover is OutgoingCover.Custom) { cover.content(); return }
  val local = item.local
  val admit = ProcessMediaEnvironment.current?.media?.let { runCatching { it.decodeAdmit() }.getOrNull() }
  val picture by produceState<ImageBitmap?>(null, local) {
    value = local?.let { file -> runCatching { decodeImage(ImageInput { file.inputStream() }, COVER_SIDE, admit).asImageBitmap() }.getOrNull() }
  }
  val decoder = LocalBlurDecoder.current
  val hash = item.blurHash
  val blurred by produceState<ImageBitmap?>(null, hash, local == null) {
    value = if (local == null && hash != null) decodeBlur(hash, decoder) else null
  }
  val radius = (cover as? OutgoingCover.BlurredLocal)?.radius
  val shown = picture ?: blurred
  if (shown != null) {
    val blur = if (radius != null && picture != null && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) Modifier.blur(radius) else Modifier
    Image(shown, "cover", Modifier.fillMaxSize().then(blur), contentScale = contentScale)
  }
}

private const val COVER_SIDE = 1024
