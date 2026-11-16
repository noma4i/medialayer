package com.noma4i.medialayer.ui

import com.noma4i.medialayer.CallHold
import com.noma4i.medialayer.MediaLayer
import com.noma4i.medialayer.MediaException
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/** What a running recording does when a call begins: it stops and ends with the in-call refusal of the core. */
internal fun interface CallListener {
  fun onCall()
}

/**
 * The call state of the process: active while at least one [MediaCallHold] lives. A recorder registers while it runs, so the first hold reaches it
 * once and a recorder that starts during a call is refused. No polling, no work at rest.
 */
internal class CallGate {
  private val lock = Any()
  private var holds = 0
  private val listeners = LinkedHashSet<CallListener>()

  val inCall: Boolean get() = synchronized(lock) { holds > 0 }

  /** False while a call runs: the listener is not kept. */
  fun register(listener: CallListener): Boolean = synchronized(lock) {
    if (holds > 0) return false
    listeners.add(listener)
    true
  }

  fun unregister(listener: CallListener) {
    synchronized(lock) { listeners.remove(listener) }
  }

  fun hold() {
    val reached = synchronized(lock) {
      holds += 1
      if (holds == 1) listeners.toList().also { listeners.clear() } else emptyList()
    }
    reached.forEach(CallListener::onCall)
  }

  fun release() {
    synchronized(lock) { if (holds > 0) holds -= 1 }
  }
}

/**
 * A call is running while this hold lives. Playback stops and `playUrl` is refused, a running [MediaVoiceRecorder] and a running video recording
 * of [MediaCamera] stop with the in-call refusal, and a new recording is refused. Releasing restores nothing: a place plays again when its owner declares play.
 */
class MediaCallHold internal constructor(private val core: CallHold, private val gate: CallGate) : AutoCloseable {
  private val released = AtomicBoolean(false)

  fun release() {
    if (!released.compareAndSet(false, true)) return
    core.release()
    gate.release()
  }

  override fun close() = release()
}

/** Starts a call: the app holds this for as long as the call runs. */
fun MediaLayer.callHold(): MediaCallHold = callHold(Media.calls)

internal fun MediaLayer.callHold(gate: CallGate): MediaCallHold {
  val core = holdCall()
  gate.hold()
  return MediaCallHold(core, gate)
}

class MediaCallAudio internal constructor(private val call: MediaCallHold, private val audio: CallAudioHold?) : AutoCloseable {
  private val closed = AtomicBoolean()
  val live: StateFlow<Boolean> = audio?.live ?: MutableStateFlow(false)
  fun setSpeaker(on: Boolean) { audio?.setSpeaker(on) }
  override fun close() {
    if (!closed.compareAndSet(false, true)) return
    audio?.close()
    call.close()
  }
}

fun MediaLayer.callAudio(): MediaCallAudio {
  val call = callHold()
  val session = Media.audio ?: return MediaCallAudio(call, null)
  val audio = session.holdCall() ?: run {
    call.close()
    throw MediaException.Unexpected("call audio refused")
  }
  return MediaCallAudio(call, audio)
}
