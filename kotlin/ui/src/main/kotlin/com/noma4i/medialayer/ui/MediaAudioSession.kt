package com.noma4i.medialayer.ui

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.emptyFlow

class MediaAudioSession internal constructor(
  val focus: MediaAudioFocus,
  port: AudioFocusPort,
  releaseDelayMs: Long = RELEASE_DELAY_MS,
  private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Default),
  private val volume: VolumePort? = null
) {
  private val demand = AudioDemand()
  private val recordingLock = Any()
  private val owner = AudioFocusOwner(port, { synchronized(demand) { demand.target(focus) } }, releaseDelayMs, scope)

  constructor(context: Context, focus: MediaAudioFocus) : this(focus, SystemAudioFocus(context), volume = SystemVolume(context))

  val isRecording: Boolean get() = synchronized(demand) { demand.recording }

  fun activateRecording(): Boolean = synchronized(recordingLock) {
    val holders = synchronized(demand) { demand.setRecording(true); demand.recordings }
    MediaTrace.emit("audio") { "recording hold holders=$holders" }
    if (owner.reconcile()) return@synchronized true

    synchronized(demand) { demand.setRecording(false) }
    owner.reconcile()
    false
  }

  fun readVolume(): MediaVolumeReading? = volume?.reading()

  fun volumeRaises(): Flow<Unit> = volume?.raises() ?: emptyFlow()

  fun deactivateRecording(): Unit = synchronized(recordingLock) {
    val holders = synchronized(demand) { demand.setRecording(false); demand.recordings }
    MediaTrace.emit("audio") { "recording release holders=$holders" }
    owner.reconcile()
  }

  internal fun setAudible(slot: UInt, value: Boolean) {
    if (synchronized(demand) { demand.setAudible(slot, value) }) MediaTrace.emit("audio") { "audible slot=$slot $value" }
    owner.reconcile()
  }

  internal fun close() {
    owner.close()
    scope.cancel()
  }

  private companion object {
    const val RELEASE_DELAY_MS = 300L
  }
}
