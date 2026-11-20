package com.noma4i.medialayer.ui

import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.SideEffect
import com.noma4i.medialayer.Kind
import com.noma4i.medialayer.OutgoingEvent
import com.noma4i.medialayer.OutgoingFailure
import com.noma4i.medialayer.OutgoingKind
import com.noma4i.medialayer.OutgoingPhase
import com.noma4i.medialayer.OutgoingState
import com.noma4i.medialayer.OutgoingUpload
import com.noma4i.medialayer.RemoteResult
import com.noma4i.medialayer.outgoingDeclare
import com.noma4i.medialayer.outgoingNext
import com.noma4i.medialayer.outgoingView
import java.io.File
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.withPermit

val OutgoingPhase.isBusy: Boolean get() = this is OutgoingPhase.Sending || this is OutgoingPhase.AwaitingRemote || this is OutgoingPhase.FetchingRemote

/** The phase as text for the semantics (`stateDescription`): sending, awaitingRemote, fetchingRemote, ready, failed.upload, failed.remote. */
val OutgoingPhase.name: String get() = when (this) {
  is OutgoingPhase.Sending -> "sending"
  is OutgoingPhase.AwaitingRemote -> "awaitingRemote"
  is OutgoingPhase.FetchingRemote -> "fetchingRemote"
  is OutgoingPhase.Ready -> "ready"
  is OutgoingPhase.Failed -> if (reason is OutgoingFailure.Upload) "failed.upload" else "failed.remote"
}

/** 0..1 for the loader: the upload progress while sending, the fetch progress while fetching, 0 otherwise. */
val OutgoingPhase.progress: Float get() = when (this) {
  is OutgoingPhase.Sending -> progress.toFloat()
  is OutgoingPhase.FetchingRemote -> progress.toFloat()
  else -> 0f
}

/**
 * The state of one outgoing item, shared by every [OutgoingMedia] of its key. The phase is decided by the core state machine, the library
 * fetches the remote copy and swaps it in when it is complete and decoded.
 */
internal class OutgoingEntry(
  val key: String,
  val kind: Kind,
  local: File?,
  val blurHash: String?,
  private val runtime: OutgoingRuntime
) {
  private val store = runtime.store()
  /** The original kept by the library under the key until `ready`, or the declared file when no store is open. */
  @Volatile var local: File? = local?.let { source -> runCatching { store?.adopt(key, source) }.getOrNull() ?: source }
    private set

  private val lock = Any()
  private var state: OutgoingState = outgoingDeclare(key, kind.toOutgoing(), local != null, blurHash)
  private val phaseState = MutableStateFlow(state.phase)
  private var fetch: Job? = null
  private var fetchedUrl: String? = null
  private var reask: Job? = null
  private var forwarding: Job? = null
  private var onScreen = 0

  /** The phase, observable outside any view. */
  val phase: OutgoingPhase get() = phaseState.value
  val phases: StateFlow<OutgoingPhase> = phaseState.asStateFlow()

  val canOpen: Boolean get() = outgoingView(synchronized(lock) { state }).canOpen

  val hasLocal: Boolean get() = state.inputs.hasLocal

  /** The remote url from the server; `null` never takes a known url back. */
  var remote: String?
    get() = synchronized(lock) { state.inputs.remote }
    set(value) = fold(OutgoingEvent.Remote(value))

  /** One call: the upload state stream of DBlayer. Duplicates and out-of-order values are folded by the core. Calling again replaces the earlier stream. */
  fun forward(upload: Flow<OutgoingUpload>): Job {
    val job = runtime.scope.launch { upload.collect { fold(OutgoingEvent.Upload(it)) } }
    synchronized(lock) { forwarding?.cancel(); forwarding = job }
    return job
  }

  fun forward(upload: OutgoingUpload) = fold(OutgoingEvent.Upload(upload))

  /** Fetches the remote again; only from `failed(.remote)`. A failed upload is re-sent by the app and re-forwarded. */
  fun retry() = fold(OutgoingEvent.Retry)

  internal fun touch() { local?.let { source -> runCatching { store?.adopt(key, source) } } }

  /** A composed view of this item is on screen: only then the "not ready yet" answer is asked again. */
  internal fun onScreen(visible: Boolean) {
    synchronized(lock) {
      onScreen = (onScreen + if (visible) 1 else -1).coerceAtLeast(0)
      if (onScreen == 0) { reask?.cancel(); reask = null } else scheduleReask()
    }
  }

  private fun fold(event: OutgoingEvent) {
    synchronized(lock) {
      state = outgoingNext(state, event)
      phaseState.value = state.phase
      if (state.phase == OutgoingPhase.Ready) release()
      val url = state.inputs.remote
      if (url != null && state.inputs.remoteResult == null && (fetchedUrl != url || fetch?.isActive != true)) startFetch(url)
      scheduleReask()
    }
  }

  private fun release() {
    reask?.cancel()
    reask = null
    runCatching { store?.release(key) }
  }

  private fun startFetch(url: String) {
    fetch?.cancel()
    fetchedUrl = url
    fetch = runtime.scope.launch {
      val result = try {
        runtime.slots.withPermit { runtime.remote.fetch(url) { progress -> report(url, RemoteResult.Fetching(progress)) } }
      } catch (error: CancellationException) {
        throw error
      } catch (error: Exception) {
        RemoteResult.Failed(null, error.message ?: error.toString())
      }
      report(url, result)
    }
  }

  private fun report(url: String, result: RemoteResult) {
    synchronized(lock) { if (state.inputs.remote != url) return }
    fold(OutgoingEvent.RemoteResult(result))
  }

  private fun scheduleReask() {
    if (onScreen == 0 || reask?.isActive == true) return
    val after = outgoingView(state).reaskAfterMs ?: return
    val url = state.inputs.remote ?: return
    reask = runtime.scope.launch {
      runtime.clock.sleep(after.toLong())
      synchronized(lock) { if (state.inputs.remote == url && onScreen > 0 && fetch?.isActive != true) startFetch(url) }
    }
  }
}

/**
 * A sent media item. The app declares it and feeds it inputs ([forward], [remote]); the phase is decided by the core state machine,
 * the library fetches the remote copy and swaps it in when it is complete and decoded. Handles of one key share one state, which is
 * readable outside any composition. The app holds no state of its own for it.
 */
class OutgoingMedia internal constructor(private val entry: OutgoingEntry) {
  constructor(key: String, kind: Kind = Kind.IMAGE, local: File? = null, blurHash: String? = null) :
    this(Outgoing.runtime.item(key) { OutgoingEntry(key, kind, local, blurHash, Outgoing.runtime) })

  /** The app's own rule for opening; the default is `ready` only. */
  @Volatile var openable: ((OutgoingPhase) -> Boolean)? = null

  val key: String get() = entry.key
  val kind: Kind get() = entry.kind
  val blurHash: String? get() = entry.blurHash

  /** The original kept by the library under the key until `ready`, or the declared file when no store is open. */
  val local: File? get() = entry.local
  val hasLocal: Boolean get() = entry.hasLocal

  /** The phase, observable outside any view. */
  val phase: OutgoingPhase get() = entry.phase

  /** The phase now and every change after it. */
  val phases: StateFlow<OutgoingPhase> get() = entry.phases
  val canOpen: Boolean get() = openable?.invoke(phase) ?: entry.canOpen

  /** The remote url from the server; `null` never takes a known url back. Setting it also means the upload is done. */
  var remote: String?
    get() = entry.remote
    set(value) { entry.remote = value }

  /** One call: the upload state stream of DBlayer. Duplicates and out-of-order values are folded by the core. Calling again replaces the earlier stream. */
  fun forward(upload: Flow<OutgoingUpload>): Job = entry.forward(upload)

  /** One upload state of DBlayer. */
  fun forward(upload: OutgoingUpload) = entry.forward(upload)

  /** Fetches the remote again; only from `failed(.remote)`. A failed upload is re-sent by the app and re-forwarded. */
  fun retry() = entry.retry()

  /** A composed view of this item is on screen: only then the "not ready yet" answer is asked again. */
  internal fun onScreen(visible: Boolean) = entry.onScreen(visible)
}

private fun Kind.toOutgoing(): OutgoingKind = when (this) {
  Kind.IMAGE -> OutgoingKind.IMAGE
  Kind.VIDEO -> OutgoingKind.VIDEO
  else -> throw IllegalArgumentException("outgoing media is an image or a video")
}

/**
 * The item of [key] from the registry, declared on first use. Same key = same item across row recycling.
 * [remote] and [upload] are inputs; the phase comes from the library.
 */
@Composable
fun rememberOutgoingMedia(key: String, kind: Kind = Kind.IMAGE, local: File? = null, blurHash: String? = null, remote: String? = null, upload: OutgoingUpload? = null): OutgoingMedia {
  val item = remember(key) { OutgoingMedia(key, kind, local, blurHash) }
  SideEffect {
    if (upload != null) item.forward(upload)
    if (remote != null) item.remote = remote
  }
  return item
}
