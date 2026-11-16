package com.noma4i.medialayer.ui

import android.content.Context
import java.util.UUID
import java.util.concurrent.atomic.AtomicBoolean
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

class MediaRecordingHold internal constructor(private val release: () -> Unit) : AutoCloseable {
  private val closed = AtomicBoolean()
  override fun close() { if (closed.compareAndSet(false, true)) release() }
}

internal class CallAudioHold(
  val live: StateFlow<Boolean>,
  private val speaker: (Boolean) -> Unit,
  private val release: () -> Unit
) : AutoCloseable {
  private val closed = AtomicBoolean()
  fun setSpeaker(on: Boolean) { if (!closed.get()) speaker(on) }
  override fun close() { if (closed.compareAndSet(false, true)) release() }
}

class MediaAudioSession internal constructor(
  val focus: MediaAudioFocus,
  port: AudioFocusPort,
  releaseDelayMs: Long = RELEASE_DELAY_MS,
  private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Default),
  private val volume: VolumePort? = null,
  private val microphone: MicrophonePort = AvailableMicrophone
) {
  private val demand = AudioDemand()
  private val recordingLock = Any()
  private val callStates = HashMap<UUID, MutableStateFlow<Boolean>>()
  private var microphoneState = MicrophoneState()
  private var failure: MicrophoneAvailability? = null
  private var externalChange: (Boolean) -> Unit = {}
  private val externalLock = Any()
  private val microphoneListeners = LinkedHashSet<() -> Unit>()
  private val availability = MutableStateFlow(MicrophoneAvailability.AVAILABLE)
  val microphoneAvailability: StateFlow<MicrophoneAvailability> = availability
  private val owner = AudioFocusOwner(
    port, { synchronized(demand) { demand.target(focus) } }, releaseDelayMs, scope,
    { synchronized(demand) { demand.communication() } },
    { value -> synchronized(demand) { callStates.values.forEach { it.value = value } }; MediaTrace.emit("audio") { "call live $value" } }
  )

  init {
    port.onChange { owner.reconcile() }
    microphone.subscribe { state, source ->
      val listeners = synchronized(recordingLock) {
        microphoneState = state
        failure = null
        updateMicrophone(source)
        microphoneListeners.toList()
      }
      publishExternal()
      listeners.forEach { it() }
    }
  }

  constructor(context: Context, focus: MediaAudioFocus) : this(focus, SystemAudioFocus(context), volume = SystemVolume(context), microphone = SystemMicrophone(context))

  val isRecording: Boolean get() = synchronized(demand) { demand.recording }

  fun holdRecording(): MediaRecordingHold? = synchronized(recordingLock) {
    if (availability.value != MicrophoneAvailability.AVAILABLE) return@synchronized null
    val id = UUID.randomUUID()
    val holders = synchronized(demand) {
      if (demand.calls.isNotEmpty()) return@synchronized null
      demand.setRecording(id, true)
      demand.recordings.size
    } ?: return@synchronized null
    MediaTrace.emit("audio") { "recording hold holders=$holders" }
    val hold = MediaRecordingHold {
      val count = synchronized(demand) { demand.setRecording(id, false); demand.recordings.size }
      MediaTrace.emit("audio") { "recording release holders=$count" }
      owner.reconcile()
      synchronized(recordingLock) { updateMicrophone("recording release") }
    }
    if (owner.reconcile()) return@synchronized hold
    hold.close()
    recordingFailed()
    null
  }

  internal fun holdCall(): CallAudioHold? {
    val result = synchronized(recordingLock) {
      val id = UUID.randomUUID()
      val live = MutableStateFlow(false)
      synchronized(demand) { demand.calls[id] = false; callStates[id] = live }
      updateMicrophone("call hold")
      MediaTrace.emit("audio") { "call hold" }
      val hold = CallAudioHold(live, { on ->
        synchronized(demand) { if (demand.calls.containsKey(id)) demand.calls[id] = on }
        MediaTrace.emit("audio") { "call speaker $on" }
        owner.reconcile()
      }, {
        synchronized(demand) { demand.calls.remove(id); callStates.remove(id); live.value = false }
        MediaTrace.emit("audio") { "call release" }
        owner.reconcile()
        microphone.foreground()
        synchronized(recordingLock) { updateMicrophone("call release") }
        publishExternal()
      })
      if (owner.reconcile()) hold else {
        synchronized(demand) { demand.calls.remove(id); callStates.remove(id); live.value = false }
        owner.reconcile()
        updateMicrophone("call refusal")
        null
      }
    }
    publishExternal()
    return result
  }

  fun readVolume(): MediaVolumeReading? = volume?.reading()

  fun volumeRaises(): Flow<Unit> = volume?.raises() ?: emptyFlow()

  internal fun recordingFailed(reason: MicrophoneAvailability = MicrophoneAvailability.OTHER_APP) = synchronized(recordingLock) {
    failure = reason
    updateMicrophone("recording failure")
  }

  internal fun onExternalCall(change: (Boolean) -> Unit) {
    synchronized(externalLock) { externalChange = change }
    publishExternal()
  }

  internal fun foreground() = microphone.foreground()

  internal fun observeMicrophone(listener: () -> Unit) = synchronized(recordingLock) { microphoneListeners.add(listener) }

  internal fun forgetMicrophone(listener: () -> Unit) = synchronized(recordingLock) { microphoneListeners.remove(listener) }

  private fun updateMicrophone(source: String) {
    val ownCall = synchronized(demand) { demand.calls.isNotEmpty() }
    val recording = synchronized(demand) { demand.recording }
    val external = microphoneState.externalCall && !ownCall
    val next = when {
      !microphoneState.permissionGranted -> MicrophoneAvailability.PERMISSION_DENIED
      ownCall -> MicrophoneAvailability.OWN_CALL
      external -> MicrophoneAvailability.EXTERNAL_CALL
      failure != null -> failure!!
      !microphoneState.inputAvailable || (microphoneState.otherRecording && !recording) -> MicrophoneAvailability.OTHER_APP
      else -> MicrophoneAvailability.AVAILABLE
    }
    if (availability.value != next) {
      availability.value = next
      MediaTrace.emit("audio") { "microphone $next source=$source" }
    }
  }

  private fun publishExternal() = synchronized(externalLock) {
    val active = synchronized(recordingLock) { microphoneState.externalCall && synchronized(demand) { demand.calls.isEmpty() } }
    externalChange(active)
  }

  internal fun setAudible(slot: UInt, value: Boolean) {
    if (synchronized(demand) { demand.setAudible(slot, value) }) MediaTrace.emit("audio") { "audible slot=$slot $value" }
    owner.reconcile()
  }

  internal fun close() {
    microphone.close()
    owner.close()
    scope.cancel()
  }

  private companion object {
    const val RELEASE_DELAY_MS = 300L
  }
}
